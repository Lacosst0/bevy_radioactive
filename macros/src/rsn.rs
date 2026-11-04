//! `rsn!`: `bsn!` with Rust control flow and `$` reads, expanding to
//! `bevy_radioactive::reactive`.
//!
//! - `$Ref<Type>(#Name)` reads `Type` on the nearest entity named `#Name`,
//!   `$Ref<Type>(entity)` on that entity, and `$Res<T>`, `$Query<..>`, .. are
//!   system parameters as written. Whatever contains a `$` is a tracked
//!   reader; in `on(..)`, `$Mut<Type>(..)` writes.
//! - `if`/`else if`/`else`, `match` and `for` (with `#[key(..)]`) in a scene
//!   list become regions (`switch`, `each`, `each_indexed`).
//! - `Comp(.. $X ..)` becomes `watch(..)` returning `Comp`;
//!   `Comp { field: .. $X .., .. }` keeps the other fields and patches the
//!   reactive ones in a `watch`.
//!
//! Names bound by `for` and `match` patterns are cloned per use (a *site*),
//! because `bsn!` moves what each value captures.

use std::sync::atomic::{AtomicUsize, Ordering};

use proc_macro2::{Delimiter, Group, Ident, Literal, Span, TokenStream, TokenTree};
use quote::{ToTokens, format_ident, quote};

use super::{Tokens, dashes, is_ident, is_punct, split_commas, stream, tokens_of, until_block};

static HOISTED: AtomicUsize = AtomicUsize::new(0);

thread_local! {
    /// The `#Name`s declared in the `rsn!` being expanded.
    static DECLARED: std::cell::RefCell<Vec<String>> = const { std::cell::RefCell::new(Vec::new()) };
    /// Errors found while expanding. They are reported on their own, without
    /// the `bsn!` call, so `bsn!` doesn't add parse errors around them.
    static ERRORS: std::cell::RefCell<Vec<TokenStream>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// Records an error at `span`; returns nothing to put in its place.
fn error(span: Span, message: &str) -> TokenStream {
    let error = quote::quote_spanned!(span=> ::core::compile_error!(#message););
    ERRORS.with(|errors| errors.borrow_mut().push(error));
    TokenStream::new()
}

/// Every `#Name` in `tokens`, deeply, except `$Ref<T>(#Name)` targets.
fn declared(tokens: &[TokenTree], out: &mut Vec<String>) {
    for (i, token) in tokens.iter().enumerate() {
        match token {
            TokenTree::Punct(p) if p.as_char() == '#' => {
                if let Some(TokenTree::Ident(name)) = tokens.get(i + 1) {
                    out.push(name.to_string());
                }
            }
            TokenTree::Group(g) => {
                let inner = tokens_of(g);
                let target = g.delimiter() == Delimiter::Parenthesis
                    && is_punct(tokens.get(i.wrapping_sub(1)), '>')
                    && matches!(inner.as_slice(), [hash, TokenTree::Ident(_)] if is_punct(Some(hash), '#'));
                if !target {
                    declared(&inner, out);
                }
            }
            _ => {}
        }
    }
}

pub(crate) fn expand(input: TokenStream, list: bool) -> TokenStream {
    let tokens: Tokens = input.into_iter().collect();
    let mut names = Vec::new();
    declared(&tokens, &mut names);
    let previous = DECLARED.with(|declared| declared.replace(names));
    let previous_errors = ERRORS.with(|errors| errors.replace(Vec::new()));
    let mut scope = Scope::default();
    let bsn = if list {
        invoke(
            "bsn_list",
            Span::call_site(),
            list_tokens(&tokens, &mut scope),
        )
    } else {
        invoke("bsn", Span::call_site(), list_item(&tokens, &mut scope))
    };
    DECLARED.with(|declared| declared.replace(previous));
    let errors = ERRORS.with(|errors| errors.replace(previous_errors));
    if !errors.is_empty() {
        return quote!({ #(#errors)* loop {} });
    }
    let prelude = scope.prelude;
    let output = quote! {{
        #[allow(unused_imports)]
        use ::bevy_radioactive::reactive::__private::*;
        #prelude
        #bsn
    }};
    if std::env::var_os("RSN_DEBUG").is_some() {
        eprintln!("{output}");
    }
    output
}

/// `bevy::scene::bsn! { body }`, spanned at `span`. `bsn!` keys entity
/// names (`#Name`) by its call site's location, so each generated call gets
/// the location of the source it comes from: names in different bodies stay
/// apart.
fn invoke(name: &str, span: Span, body: TokenStream) -> TokenStream {
    let name = Ident::new(name, span);
    let mut group = Group::new(Delimiter::Brace, body);
    group.set_span(span);
    quote::quote_spanned!(span=> ::bevy::scene::#name! #group)
}

// ---------------------------------------------------------------------------
// Scopes: names bound by enclosing `for`/`match` patterns
// ---------------------------------------------------------------------------

#[derive(Default)]
struct Scope {
    /// `let __rsn_hN = clone(&name);` for each site, run once per render.
    prelude: TokenStream,
    /// Names bound by enclosing patterns, visible here.
    bound: Vec<Ident>,
}

impl Scope {
    fn child(&self, extra: Vec<Ident>) -> Scope {
        let mut bound = self.bound.clone();
        for name in extra {
            if !bound.contains(&name) {
                bound.push(name);
            }
        }
        Scope {
            prelude: TokenStream::new(),
            bound,
        }
    }

    fn mentions(&self, tokens: &[TokenTree]) -> bool {
        !self.bound.is_empty() && mentions(tokens, &self.bound)
    }

    /// `let name = clone(&hoisted);` for each bound name `tokens` uses, with
    /// the hoisted clones declared in the prelude.
    fn site(&mut self, tokens: &[TokenTree]) -> TokenStream {
        let mut bindings = TokenStream::new();
        for name in self.bound.clone() {
            if !mentions(tokens, std::slice::from_ref(&name)) {
                continue;
            }
            let hoisted = format_ident!("__rsn_h{}", HOISTED.fetch_add(1, Ordering::Relaxed));
            quote!(let #hoisted = ::core::clone::Clone::clone(&#name);)
                .to_tokens(&mut self.prelude);
            quote!(#[allow(unused_variables)] let #name = ::core::clone::Clone::clone(&#hoisted);)
                .to_tokens(&mut bindings);
        }
        bindings
    }

    /// `{ site expr }`.
    fn capture(&mut self, tokens: TokenStream, used: &[TokenTree]) -> TokenStream {
        if !self.mentions(used) {
            return tokens;
        }
        let site = self.site(used);
        quote!({ #site #tokens })
    }
}

fn mentions(tokens: &[TokenTree], names: &[Ident]) -> bool {
    tokens.iter().any(|token| match token {
        TokenTree::Ident(ident) => names.contains(ident),
        TokenTree::Group(g) => mentions(&tokens_of(g), names),
        _ => false,
    })
}

/// Names a pattern binds (approximate: lowercase identifiers that aren't
/// keywords, path segments or field names).
fn bindings(pattern: &[TokenTree]) -> Vec<Ident> {
    const KEYWORDS: &[&str] = &[
        "mut", "ref", "box", "crate", "self", "super", "_", "true", "false",
    ];
    let mut out = Vec::new();
    for (i, token) in pattern.iter().enumerate() {
        match token {
            TokenTree::Ident(ident) => {
                let name = ident.to_string();
                let next = pattern.get(i + 1);
                let path = is_punct(next, ':') || matches!(next, Some(TokenTree::Group(_)));
                if name.starts_with(|c: char| c.is_lowercase() || c == '_')
                    && !KEYWORDS.contains(&name.as_str())
                    && !path
                {
                    out.push(ident.clone());
                }
            }
            TokenTree::Group(g) => out.extend(bindings(&tokens_of(g))),
            _ => {}
        }
    }
    out
}

// ---------------------------------------------------------------------------
// `$` reads
// ---------------------------------------------------------------------------

/// Where a `$` read comes from.
enum Kind {
    /// `$Ref<Type>(#Name)` or `$Ref<Type>("Name")`: `Type` on the nearest
    /// entity named `Name`.
    Named(String),
    /// `$Res<T>`, `$Query<..>`, ..: that system parameter, as written.
    Exact,
    /// `$Ref<Type>(entity)`: `Type` on that entity.
    Entity(TokenStream),
}

struct Param {
    key: String,
    name: Ident,
    ty: TokenStream,
    kind: Kind,
}

/// The `$` reads of one closure, one parameter per distinct read.
#[derive(Default)]
struct Params {
    params: Vec<Param>,
    /// In an `on(..)` handler, where `$Mut<T>(..)` writes.
    write: bool,
}

impl Params {
    fn param(&mut self, ty: &[TokenTree], kind: Kind) -> Ident {
        // In a handler, `$Res<T>` and `$ResMut<T>` share one `ResMut<T>`:
        // two parameters would conflict.
        let resource = match ty {
            [TokenTree::Ident(res), TokenTree::Punct(lt), inner @ ..]
                if self.write
                    && matches!(kind, Kind::Exact)
                    && lt.as_char() == '<'
                    && (res == "Res" || res == "ResMut") =>
            {
                Some((res == "ResMut", inner))
            }
            _ => None,
        };
        let key = match (&kind, resource) {
            (_, Some((_, inner))) => format!("resource {}", stream(inner)),
            (Kind::Named(entity), _) => format!("#{entity}.{}", stream(ty)),
            (Kind::Exact, _) => format!("exact {}", stream(ty)),
            (Kind::Entity(entity), _) => format!("({entity}).{}", stream(ty)),
        };
        if let Some(param) = self.params.iter_mut().find(|p| p.key == key) {
            if let Some((true, _)) = resource {
                param.ty = stream(ty);
            }
            return param.name.clone();
        }
        let name = format_ident!("__rsn_{}", self.params.len());
        self.params.push(Param {
            key,
            name: name.clone(),
            ty: stream(ty),
            kind,
        });
        name
    }

    fn by_entity(&self) -> bool {
        self.params
            .iter()
            .any(|p| matches!(p.kind, Kind::Entity(_) | Kind::Named(_)))
    }

    fn named(&self) -> bool {
        self.params.iter().any(|p| matches!(p.kind, Kind::Named(_)))
    }

    /// Closure parameters, ending with a comma when not empty.
    fn closure_params(&self, write: bool) -> TokenStream {
        let mut out = TokenStream::new();
        for (index, param) in self.params.iter().enumerate() {
            let Param { name, ty, kind, .. } = param;
            match (kind, write) {
                (Kind::Exact, _) => quote!(#[allow(unused_mut)] mut #name: #ty,),
                (Kind::Entity(_) | Kind::Named(_), false) => {
                    let query = format_ident!("__rsn_q{index}");
                    quote!(#query: ::bevy::prelude::Query<::bevy::prelude::Ref<#ty>>,)
                }
                (Kind::Entity(_) | Kind::Named(_), true) => {
                    let query = format_ident!("__rsn_q{index}");
                    quote!(mut #query: ::bevy::prelude::Query<&mut #ty>,)
                }
            }
            .to_tokens(&mut out);
        }
        if self.named() {
            quote! {
                __rsn_host: ::bevy::prelude::Entity,
                mut __rsn_names: ::bevy_radioactive::reactive::Untracked<__rsn_Names>,
            }
            .to_tokens(&mut out);
        }
        out
    }

    /// Looks up `$(entity).Type` reads; a reader skips its run while one is
    /// missing, a handler does nothing.
    fn lookups(&self, write: bool) -> TokenStream {
        let mut out = TokenStream::new();
        for (index, param) in self.params.iter().enumerate() {
            let name = &param.name;
            let query = format_ident!("__rsn_q{index}");
            let ty = &param.ty;
            let entity = match &param.kind {
                Kind::Entity(entity) => entity.clone(),
                Kind::Named(entity) => {
                    // Warns once in debug builds when nothing matches.
                    let get = if write {
                        quote!(#query.get_mut(__rsn_e).ok())
                    } else {
                        quote!(#query.get(__rsn_e).ok())
                    };
                    let found = quote! {{
                        let __rsn_e = __rsn_names.find(__rsn_host, #entity);
                        let __rsn_found = #get;
                        __rsn_names.check::<#ty, _>(__rsn_found, #entity)
                    }};
                    if write {
                        quote!(let ::core::option::Option::Some(mut #name) = (#found) else { return; };)
                    } else {
                        quote!(let #name = #found?;)
                    }
                    .to_tokens(&mut out);
                    continue;
                }
                Kind::Exact => continue,
            };
            if write {
                quote!(let ::core::result::Result::Ok(mut #name) = #query.get_mut(#entity) else { return; };)
            } else {
                quote!(let #name = ::core::result::Result::ok(#query.get(#entity))?;)
            }
            .to_tokens(&mut out);
        }
        out
    }

    /// A reader closure: `move |params| -> ret { body }`, wrapped so it
    /// skips runs while a `$(entity).Type` is missing.
    fn reader(
        &self,
        extra: TokenStream,
        ret: Option<TokenStream>,
        body: TokenStream,
    ) -> TokenStream {
        let params = self.closure_params(false);
        if !self.by_entity() {
            let ret = ret.map(|ret| quote!(-> #ret));
            return quote!(move |#params #extra| #ret { #body });
        }
        let lookups = self.lookups(false);
        let ret = ret.unwrap_or_else(|| quote!(_));
        let skip = private("skip");
        quote! {
            #skip(move |#params #extra| -> ::core::option::Option<#ret> {
                #lookups
                ::core::option::Option::Some({ #body })
            })
        }
    }
}

const DOLLAR_HELP: &str = "expected a Bevy system parameter: `$Ref<T>(#Name)` or \
     `$Ref<T>(entity)` for a component on an entity (`$Mut<T>(..)` to write it in `on(..)`), \
     or `$Res<T>`, `$ResMut<T>`, `$Local<T>`, `$Query<..>`, `$Single<..>`, `$Commands`";

/// Parses a `$` read at `tokens[i]` (the `$`): returns the parameter name
/// and the index after the read.
fn dollar(
    tokens: &[TokenTree],
    i: usize,
    params: &mut Params,
) -> Result<(Ident, usize), (Span, &'static str)> {
    let span = tokens[i].span();
    let mut j = i + 1;
    let start = j;
    if is_punct(tokens.get(j), ':') && is_punct(tokens.get(j + 1), ':') {
        j += 2;
    }
    loop {
        if !matches!(tokens.get(j), Some(TokenTree::Ident(_))) {
            break;
        }
        j += 1;
        if is_punct(tokens.get(j), ':')
            && is_punct(tokens.get(j + 1), ':')
            && matches!(tokens.get(j + 2), Some(TokenTree::Ident(_)))
        {
            j += 2;
        } else {
            break;
        }
    }
    // `$Commands`: the one common parameter without generics.
    if j == start + 1 && is_ident(tokens.get(start), "Commands") && !is_punct(tokens.get(j), '<') {
        return Ok((params.param(&tokens[start..j], Kind::Exact), j));
    }
    if j == start || !is_punct(tokens.get(j), '<') {
        return Err((span, DOLLAR_HELP));
    }
    let path_end = j;
    let mut depth = 0usize;
    while j < tokens.len() {
        if is_punct(tokens.get(j), '<') {
            depth += 1;
        } else if is_punct(tokens.get(j), '>') && !is_punct(tokens.get(j - 1), '-') {
            depth -= 1;
            if depth == 0 {
                break;
            }
        }
        j += 1;
    }
    if depth != 0 {
        return Err((span, DOLLAR_HELP));
    }
    j += 1;
    // `$Ref<T>(target)` / `$Mut<T>(target)`: a component on one entity.
    let path = &tokens[start..path_end];
    let (is_ref, is_mut) = (
        path.len() == 1 && is_ident(path.first(), "Ref"),
        path.len() == 1 && is_ident(path.first(), "Mut"),
    );
    if !is_ref && !is_mut {
        return Ok((params.param(&tokens[start..j], Kind::Exact), j));
    }
    if is_mut && !params.write {
        return Err((
            span,
            "`$Mut<T>(..)` writes, so it only works in `on(..)`; read with `$Ref<T>(..)`",
        ));
    }
    let Some(TokenTree::Group(target)) = tokens.get(j) else {
        return Err((
            span,
            "write the entity: `$Ref<T>(#Name)` for the nearest entity named `#Name`, or `$Ref<T>(entity)`",
        ));
    };
    if target.delimiter() != Delimiter::Parenthesis {
        return Err((target.span(), DOLLAR_HELP));
    }
    let ty = &tokens[path_end + 1..j - 1];
    let inside = tokens_of(target);
    let kind = match inside.as_slice() {
        [hash, TokenTree::Ident(name)] if is_punct(Some(hash), '#') => {
            let name = name.to_string();
            if !DECLARED.with(|declared| declared.borrow().contains(&name)) {
                return Err((
                    target.span(),
                    "this `#Name` isn't declared in this `rsn!`; check the spelling, or, for an \
                     entity named in another scene, look it up at runtime with `$Ref<T>(\"Name\")`",
                ));
            }
            Kind::Named(name)
        }
        [TokenTree::Literal(literal)] if literal.to_string().starts_with('"') => {
            let text = literal.to_string();
            Kind::Named(text[1..text.len() - 1].to_string())
        }
        _ => Kind::Entity(target.stream()),
    };
    Ok((params.param(ty, kind), j + 1))
}

const MISPLACED_DOLLAR: &str =
    "`$` reads work in component values, `on(..)` handlers and `if`/`match`/`for` headers";

fn has_dollar(tokens: &[TokenTree]) -> bool {
    tokens.iter().enumerate().any(|(i, token)| match token {
        TokenTree::Punct(p) => p.as_char() == '$',
        TokenTree::Group(g) => !nested_rsn(tokens, i) && has_dollar(&tokens_of(g)),
        _ => false,
    })
}

/// Whether the group at `index` is the body of a nested `rsn!`/`rsn_list!`,
/// whose `$` reads belong to it.
fn nested_rsn(tokens: &[TokenTree], index: usize) -> bool {
    index >= 2
        && is_punct(tokens.get(index - 1), '!')
        && (is_ident(tokens.get(index - 2), "rsn") || is_ident(tokens.get(index - 2), "rsn_list"))
}

/// Rewrites each `$` read to its parameter, deeply.
fn reads(tokens: &[TokenTree], params: &mut Params) -> TokenStream {
    let mut out = TokenStream::new();
    let mut i = 0;
    while i < tokens.len() {
        match &tokens[i] {
            TokenTree::Punct(p) if p.as_char() == '$' => match dollar(tokens, i, params) {
                Ok((name, next)) => {
                    name.to_tokens(&mut out);
                    i = next;
                }
                Err((span, message)) => return error(span, message),
            },
            TokenTree::Group(g) if !nested_rsn(tokens, i) => {
                let mut group = Group::new(g.delimiter(), reads(&tokens_of(g), params));
                group.set_span(g.span());
                group.to_tokens(&mut out);
                i += 1;
            }
            other => {
                other.to_tokens(&mut out);
                i += 1;
            }
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Entities
// ---------------------------------------------------------------------------

fn private(name: &str) -> Ident {
    Ident::new(&format!("__rsn_{name}"), Span::call_site())
}

/// Transforms the parts of one BSN entity.
fn entity(tokens: &[TokenTree], scope: &mut Scope) -> TokenStream {
    let mut out = TokenStream::new();
    let mut i = 0;
    while i < tokens.len() {
        let starts_path = matches!(tokens[i], TokenTree::Ident(_))
            || (is_punct(tokens.get(i), ':') && is_punct(tokens.get(i + 1), ':'));
        let named = i > 0 && is_punct(tokens.get(i - 1), '#');
        if !starts_path || named {
            match &tokens[i] {
                TokenTree::Group(g) if has_dollar(&tokens_of(g)) => {
                    error(g.span(), MISPLACED_DOLLAR).to_tokens(&mut out);
                }
                TokenTree::Group(g)
                    if g.delimiter() == Delimiter::Brace && scope.mentions(&tokens_of(g)) =>
                {
                    let inner = tokens_of(g);
                    let site = scope.site(&inner);
                    let inner = stream(&inner);
                    Group::new(Delimiter::Brace, quote!({ #site #inner })).to_tokens(&mut out);
                }
                other => other.to_tokens(&mut out),
            }
            i += 1;
            continue;
        }

        let start = i;
        if is_punct(tokens.get(i), ':') {
            i += 2;
        }
        while let Some(TokenTree::Ident(_)) = tokens.get(i) {
            i += 1;
            if is_punct(tokens.get(i), ':') && is_punct(tokens.get(i + 1), ':') {
                i += 2;
            } else {
                break;
            }
        }
        let path = &tokens[start..i];
        let idents: Vec<String> = path
            .iter()
            .filter_map(|t| match t {
                TokenTree::Ident(ident) => Some(ident.to_string()),
                _ => None,
            })
            .collect();
        let upper = |s: &String| s.starts_with(char::is_uppercase);
        // `Type`, `Type { .. }`, `Type(..)` or `Enum::Variant`, as in `bsn!`.
        let is_component = idents.last().is_some_and(upper);
        let is_on = path.len() == 1 && is_ident(path.first(), "on");

        if let Some(TokenTree::Group(g)) = tokens.get(i)
            && g.delimiter() == Delimiter::Bracket
        {
            stream(path).to_tokens(&mut out);
            Group::new(Delimiter::Bracket, list_tokens(&tokens_of(g), scope)).to_tokens(&mut out);
            i += 1;
            continue;
        }
        let group = match tokens.get(i) {
            Some(TokenTree::Group(g)) if g.delimiter() != Delimiter::None => {
                i += 1;
                Some(g.clone())
            }
            _ => None,
        };
        // `.method(..)` / `.field` chained on the value, as `bsn!` allows.
        let chain_start = i;
        while is_punct(tokens.get(i), '.') && !is_punct(tokens.get(i + 1), '.') {
            i += 1;
            if matches!(
                tokens.get(i),
                Some(TokenTree::Ident(_) | TokenTree::Literal(_))
            ) {
                i += 1;
            }
            if matches!(tokens.get(i), Some(TokenTree::Group(g)) if g.delimiter() == Delimiter::Parenthesis)
            {
                i += 1;
            }
        }
        let chain = &tokens[chain_start..i];
        let content = group.as_ref().map(tokens_of).unwrap_or_default();

        if has_dollar(&content) || has_dollar(chain) {
            if start > 0
                && (is_punct(tokens.get(start - 1), '@') || is_punct(tokens.get(start - 1), ':'))
            {
                error(
                    tokens[start].span(),
                    "a scene's arguments are read once; read `$` inside the scene function's own \
                     `rsn!`, or pick the scene with `if`/`match`",
                );
                continue;
            }
            match &group {
                Some(g) if is_on && chain.is_empty() => handler(g, scope).to_tokens(&mut out),
                Some(g) if is_component && chain.is_empty() => {
                    reactive_component(path, g, scope).to_tokens(&mut out);
                }
                // `Type::new(.. $X ..)`, `function(..)`, `Type(..).with(..)`:
                // a watch that rebuilds the value.
                _ => watched_expression(&tokens[start..i], scope).to_tokens(&mut out),
            }
            continue;
        }
        stream(path).to_tokens(&mut out);
        if let Some(g) = group {
            if scope.mentions(&content) {
                captured_args(&g, is_component, scope).to_tokens(&mut out);
            } else {
                g.to_tokens(&mut out);
            }
        }
        stream(chain).to_tokens(&mut out);
    }
    out
}

/// `Type::new(.. $X ..)` (or any value expression with `$` reads) →
/// `watch(|x: Ref<X>| Type::new(..))`, writing the component it returns.
fn watched_expression(expression: &[TokenTree], scope: &mut Scope) -> TokenStream {
    let site = scope.site(expression);
    let mut params = Params::default();
    let expression: Tokens = expression
        .iter()
        .map(|token| match token {
            // `f({ value })`: BSN's braced value is a plain argument in Rust.
            TokenTree::Group(g) if g.delimiter() == Delimiter::Parenthesis => {
                let args = split_commas(&tokens_of(g));
                let args = args.iter().map(|arg| unbrace(arg));
                let mut group = Group::new(Delimiter::Parenthesis, quote!(#(#args),*));
                group.set_span(g.span());
                TokenTree::Group(group)
            }
            other => other.clone(),
        })
        .collect();
    let body = reads(&expression, &mut params);
    let reader = params.reader(TokenStream::new(), None, body);
    let watch = private("watch");
    quote!(#watch({ #site #reader }))
}

/// Arguments or fields that read bound names through their own clones.
fn captured_args(group: &Group, is_component: bool, scope: &mut Scope) -> Group {
    let parts = split_commas(&tokens_of(group));
    let parts: Vec<TokenStream> = parts
        .iter()
        .map(|part| {
            if !scope.mentions(part) {
                return stream(part);
            }
            let (name, value) = match field_split(part, group.delimiter()) {
                Some((name, value)) => (Some(name), value),
                None => (None, &part[..]),
            };
            let site = scope.site(value);
            let value = unbrace(value);
            let value = if is_component {
                // A BSN value: `{expr}`.
                quote!({ { #site #value } })
            } else {
                quote!({ #site #value })
            };
            match name {
                Some(name) => quote!(#name: #value),
                None => value,
            }
        })
        .collect();
    let mut out = Group::new(group.delimiter(), quote!(#(#parts),*));
    out.set_span(group.span());
    out
}

/// A BSN `{expr}` value as a Rust expression: the braces are dropped unless
/// the block has statements.
fn unbrace(value: &[TokenTree]) -> TokenStream {
    match value {
        [TokenTree::Group(g)]
            if g.delimiter() == Delimiter::Brace
                && !tokens_of(g).iter().any(|t| is_punct(Some(t), ';')) =>
        {
            g.stream()
        }
        _ => stream(value),
    }
}

/// `name: value` in a braced component.
fn field_split(field: &[TokenTree], delimiter: Delimiter) -> Option<(Ident, &[TokenTree])> {
    if delimiter != Delimiter::Brace {
        return None;
    }
    let TokenTree::Ident(name) = field.first()? else {
        return None;
    };
    if is_punct(field.get(1), ':') && !is_punct(field.get(2), ':') {
        Some((name.clone(), &field[2..]))
    } else {
        None
    }
}

/// `Comp(.. $X ..)` → `watch(|x: Ref<X>| Comp(..))`.
/// `Comp { a: .. $X .., b: .. }` → `Comp { b: .. } watch(|x: Ref<X>, c: &mut Comp| c.a = ..)`.
fn reactive_component(path: &[TokenTree], group: &Group, scope: &mut Scope) -> TokenStream {
    let watch = private("watch");
    let ty = stream(path);
    let content = tokens_of(group);
    let site = scope.site(&content);
    let mut params = Params::default();
    if group.delimiter() == Delimiter::Parenthesis {
        let args = split_commas(&content)
            .iter()
            .map(|arg| converted(arg, &mut params))
            .collect::<Vec<_>>();
        let reader = params.reader(TokenStream::new(), None, quote!(#ty(#(#args),*)));
        return quote! {
            #watch({ #site #reader })
        };
    }
    if group.delimiter() != Delimiter::Brace {
        return error(group.span(), "expected `Type(..)` or `Type { field: .. }`");
    }
    let mut kept = Vec::new();
    let mut assignments = TokenStream::new();
    for field in split_commas(&content) {
        if !has_dollar(&field) {
            kept.push(field);
            continue;
        }
        let Some((name, value)) = field_split(&field, Delimiter::Brace) else {
            return error(group.span(), "expected `field: value` around a `$` read");
        };
        assign(quote!(__rsn_c.#name), value, &mut params).to_tokens(&mut assignments);
    }
    let kept = if kept.is_empty() {
        TokenStream::new()
    } else {
        let kept = captured_args(
            &Group::new(Delimiter::Brace, quote!(#(#(#kept)*),*)),
            true,
            scope,
        );
        kept.into_token_stream()
    };
    let reader = params.reader(quote!(__rsn_c: &mut #ty), None, assignments);
    quote! {
        #ty #kept
        #watch({ #site #reader })
    }
}

/// `place = value;`. A nested struct value, `UiRect { left: .. }`, patches
/// only the fields it names, as in `bsn!`.
fn assign(place: TokenStream, value: &[TokenTree], params: &mut Params) -> TokenStream {
    if let [path @ .., TokenTree::Group(fields)] = value
        && fields.delimiter() == Delimiter::Brace
        && is_struct_path(path)
    {
        let fields = split_commas(&tokens_of(fields));
        let named: Option<Vec<_>> = fields
            .iter()
            .map(|field| match field.as_slice() {
                [TokenTree::Ident(name)] => Some((name.clone(), vec![field[0].clone()])),
                _ => {
                    field_split(field, Delimiter::Brace).map(|(name, value)| (name, value.to_vec()))
                }
            })
            .collect();
        if let Some(named) = named {
            return named
                .iter()
                .map(|(name, value)| assign(quote!(#place.#name), value, params))
                .collect();
        }
    }
    let value = converted(value, params);
    quote!(#place = #value;)
}

/// A BSN value as Rust, converted with `Into` like `bsn!` does; unsuffixed
/// number literals stay as they are, so they infer the field's type.
fn converted(value: &[TokenTree], params: &mut Params) -> TokenStream {
    let number = |t: &TokenTree| matches!(t, TokenTree::Literal(l) if !l.to_string().starts_with(['"', '\'', 'b', 'c', 'r']));
    match value {
        [literal] | [TokenTree::Punct(_), literal]
            if number(literal) && (value.len() == 1 || is_punct(value.first(), '-')) =>
        {
            stream(value)
        }
        _ => {
            let value = reads(&unbrace(value).into_iter().collect::<Tokens>(), params);
            quote!(::core::convert::Into::into(#value))
        }
    }
}

/// `Struct` or `path::to::Struct`, but not `Enum::Variant`.
fn is_struct_path(path: &[TokenTree]) -> bool {
    let idents: Vec<String> = path
        .iter()
        .filter_map(|t| match t {
            TokenTree::Ident(ident) => Some(ident.to_string()),
            TokenTree::Punct(p) if p.as_char() == ':' => None,
            _ => Some(String::new()),
        })
        .collect();
    let upper = |s: &String| s.starts_with(char::is_uppercase);
    match idents.as_slice() {
        [.., parent, last] => upper(last) && !upper(parent),
        [last] => upper(last),
        [] => false,
    }
}

/// Splits on top-level `,`, treating `<..>` as nesting.
fn split_params(tokens: &[TokenTree]) -> Vec<Tokens> {
    let mut parts = vec![Vec::new()];
    let mut depth = 0usize;
    for (i, token) in tokens.iter().enumerate() {
        match token {
            TokenTree::Punct(p) if p.as_char() == '<' => depth += 1,
            TokenTree::Punct(p)
                if p.as_char() == '>' && !is_punct(tokens.get(i.wrapping_sub(1)), '-') =>
            {
                depth = depth.saturating_sub(1);
            }
            TokenTree::Punct(p) if p.as_char() == ',' && depth == 0 => {
                parts.push(Vec::new());
                continue;
            }
            _ => {}
        }
        parts.last_mut().unwrap().push(token.clone());
    }
    if parts.last().is_some_and(Vec::is_empty) {
        parts.pop();
    }
    parts
}

/// `on(|e: On<E>, rest..| body with $X)` → `handle(|e: &E, rest.., x: &mut X| body)`.
fn handler(group: &Group, scope: &mut Scope) -> TokenStream {
    let content = tokens_of(group);
    let mut i = 0;
    if is_ident(content.first(), "move") {
        i += 1;
    }
    if !is_punct(content.get(i), '|') {
        return error(group.span(), "expected `on(|event: On<E>, ..| ..)`");
    }
    let Some(close) = (i + 1..content.len()).find(|&j| is_punct(content.get(j), '|')) else {
        return error(group.span(), "expected `on(|event: On<E>, ..| ..)`");
    };
    let mut closure_params = split_params(&content[i + 1..close]);
    let body = &content[close + 1..];
    if closure_params.is_empty() {
        return error(group.span(), "expected `on(|event: On<E>, ..| ..)`");
    }
    let first = closure_params.remove(0);
    let Some(colon) = first.iter().position(|t| is_punct(Some(t), ':')) else {
        return error(
            group.span(),
            "write the event type: `on(|event: On<E>| ..)`",
        );
    };
    let pattern = stream(&first[..colon]);
    let ty = &first[colon + 1..];
    let event = match ty {
        [TokenTree::Ident(on), lt, inner @ .., gt]
            if on == "On" && is_punct(Some(lt), '<') && is_punct(Some(gt), '>') =>
        {
            stream(inner)
        }
        _ => {
            return error(
                group.span(),
                "write the event type: `on(|event: On<E>| ..)`",
            );
        }
    };
    let site = scope.site(&content);
    let mut params = Params {
        params: Vec::new(),
        write: true,
    };
    let body = reads(body, &mut params);
    let rest = closure_params.iter().map(|p| stream(p));
    let writers = params.closure_params(true);
    let lookups = params.lookups(true);
    let handle = private("handle");
    quote! {
        #handle({ #site move |#pattern: &#event, #(#rest,)* #writers| { #lookups #body; } })
    }
}

// ---------------------------------------------------------------------------
// Scene lists and control flow
// ---------------------------------------------------------------------------

fn list_tokens(tokens: &[TokenTree], scope: &mut Scope) -> TokenStream {
    let mut joined = TokenStream::new();
    for (index, item) in split_list(tokens).iter().enumerate() {
        if index > 0 {
            dashes().to_tokens(&mut joined);
        }
        list_item(item, scope).to_tokens(&mut joined);
    }
    joined
}

/// Splits a scene list into items, like `bsn_list!`: items are separated by
/// `,` or `--`. An `if`/`match`/`for` item ends after its last block, so
/// commas in its header (`$Query<A, B>`) stay in it.
fn split_list(tokens: &[TokenTree]) -> Vec<Tokens> {
    let separator = |i: usize| {
        is_punct(tokens.get(i), ',')
            || (is_punct(tokens.get(i), '-') && is_punct(tokens.get(i + 1), '-'))
    };
    let skip_separator = |i: usize| {
        if is_punct(tokens.get(i), ',') {
            i + 1
        } else if separator(i) {
            i + 2
        } else {
            i
        }
    };
    let block_after =
        |from: usize| until_block(tokens, from).map_or(tokens.len(), |(.., next)| next);
    let mut items = Vec::new();
    let mut i = skip_separator(0);
    while i < tokens.len() {
        let start = i;
        let mut j = i;
        if is_punct(tokens.get(j), '#')
            && matches!(tokens.get(j + 1), Some(TokenTree::Group(g)) if g.delimiter() == Delimiter::Bracket)
        {
            j += 2;
        }
        if is_ident(tokens.get(j), "if") {
            j = block_after(j + 1);
            while is_ident(tokens.get(j), "else") {
                if is_ident(tokens.get(j + 1), "if") {
                    j = block_after(j + 2);
                } else {
                    j = (j + 2).min(tokens.len());
                    break;
                }
            }
        } else if is_ident(tokens.get(j), "match") {
            j = block_after(j + 1);
        } else if is_ident(tokens.get(j), "for") {
            // After `in`: the pattern may have braces, and the iterator may
            // be a block.
            let after = j + 1;
            j = (after..tokens.len())
                .find(|&k| is_ident(tokens.get(k), "in"))
                .map_or(tokens.len(), |k| block_after(k + 1));
        } else {
            while j < tokens.len() && !separator(j) {
                j += 1;
            }
        }
        items.push(tokens[start..j].to_vec());
        i = skip_separator(j);
    }
    items
}

fn list_item(item: &[TokenTree], scope: &mut Scope) -> TokenStream {
    let key = match (item.first(), item.get(1)) {
        (Some(TokenTree::Punct(p)), Some(TokenTree::Group(g)))
            if p.as_char() == '#' && g.delimiter() == Delimiter::Bracket =>
        {
            let attr = tokens_of(g);
            if !is_ident(attr.first(), "key") {
                None
            } else if let Some(TokenTree::Group(args)) = attr.get(1) {
                Some(tokens_of(args))
            } else {
                return error(g.span(), "expected `#[key(expr)]`");
            }
        }
        _ => None,
    };
    let rest = if key.is_some() { &item[2..] } else { item };
    if is_ident(rest.first(), "for") {
        for_item(rest, key.as_deref(), scope)
    } else if key.is_some() {
        error(item[0].span(), "`#[key(..)]` must precede a `for` loop")
    } else if is_ident(rest.first(), "if") {
        if_item(rest, scope)
    } else if is_ident(rest.first(), "match") {
        match_item(rest, scope)
    } else {
        entity(rest, scope)
    }
}

/// A branch's content: `{ prelude Box::new(bsn_list! { .. }) }`, with the
/// names its pattern binds.
fn rendered(body: &Group, scope: &Scope, bound: Vec<Ident>) -> TokenStream {
    let mut inner = scope.child(bound);
    let list = list_tokens(&tokens_of(body), &mut inner);
    let prelude = inner.prelude;
    let bsn_list = invoke("bsn_list", body.span(), list);
    quote! {{
        #prelude
        ::std::boxed::Box::new(#bsn_list) as ::std::boxed::Box<dyn ::bevy::scene::SceneList>
    }}
}

fn if_item(item: &[TokenTree], scope: &mut Scope) -> TokenStream {
    let mut branches: Vec<(Option<Tokens>, Group)> = Vec::new();
    let mut i = 0;
    loop {
        let Some((cond, body, next)) = until_block(item, i + 1) else {
            return error(item[i].span(), "expected `if condition { .. }`");
        };
        branches.push((Some(cond), body));
        i = next;
        if !is_ident(item.get(i), "else") {
            break;
        }
        if is_ident(item.get(i + 1), "if") {
            i += 1;
            continue;
        }
        match item.get(i + 1) {
            Some(TokenTree::Group(g)) if g.delimiter() == Delimiter::Brace => {
                branches.push((None, g.clone()));
                i += 2;
                break;
            }
            _ => return error(item[i].span(), "expected `else { .. }`"),
        }
    }
    if i < item.len() {
        return error(item[i].span(), "unexpected tokens after `if`");
    }

    let mut params = Params::default();
    let mut selector = TokenStream::new();
    let mut cond_tokens = Tokens::new();
    for (index, (cond, _)) in branches.iter().enumerate() {
        if index > 0 {
            quote!(else).to_tokens(&mut selector);
        }
        let index = Literal::usize_unsuffixed(index);
        match cond {
            Some(cond) => {
                cond_tokens.extend(cond.iter().cloned());
                let cond = reads(cond, &mut params);
                quote!(if #cond { ::core::option::Option::Some(#index) }).to_tokens(&mut selector);
            }
            None => quote!({ ::core::option::Option::Some(#index) }).to_tokens(&mut selector),
        }
    }
    if branches.last().is_some_and(|(cond, _)| cond.is_some()) {
        quote!(else { ::core::option::Option::None }).to_tokens(&mut selector);
    }
    let key = scope.capture(
        params.reader(
            TokenStream::new(),
            Some(quote!(::core::option::Option<usize>)),
            selector,
        ),
        &cond_tokens,
    );
    let body_tokens: Tokens = branches
        .iter()
        .flat_map(|(_, body)| body.stream())
        .collect();
    let arms = branches.iter().enumerate().map(|(index, (_, body))| {
        let index = Literal::usize_unsuffixed(index);
        let body = rendered(body, scope, Vec::new());
        quote!(::core::option::Option::Some(#index) => #body,)
    });
    let render = scope.capture(
        quote! {
            move |__rsn_branch: &::core::option::Option<usize>| -> ::std::boxed::Box<dyn ::bevy::scene::SceneList> {
                match *__rsn_branch {
                    #(#arms)*
                    _ => ::std::boxed::Box::new(()),
                }
            }
        },
        &body_tokens,
    );
    let switch = private("switch");
    quote!(@#switch(#key, #render))
}

fn match_item(item: &[TokenTree], scope: &mut Scope) -> TokenStream {
    let Some((scrutinee, arms_group, next)) = until_block(item, 1) else {
        return error(item[0].span(), "expected `match value { .. }`");
    };
    if next < item.len() {
        return error(item[next].span(), "unexpected tokens after `match`");
    }
    let arms_tokens = tokens_of(&arms_group);

    // `pattern => body,` where a body is `{ .. }` or tokens up to `,`.
    let mut arms: Vec<(Tokens, Group)> = Vec::new();
    let mut i = 0;
    while i < arms_tokens.len() {
        let Some(arrow) = (i..arms_tokens.len())
            .find(|&j| is_punct(arms_tokens.get(j), '=') && is_punct(arms_tokens.get(j + 1), '>'))
        else {
            return error(arms_tokens[i].span(), "expected `pattern => scene`");
        };
        let pattern = arms_tokens[i..arrow].to_vec();
        let start = arrow + 2;
        let (body, end) = match arms_tokens.get(start) {
            Some(TokenTree::Group(g)) if g.delimiter() == Delimiter::Brace => {
                (g.clone(), start + 1)
            }
            _ => {
                let end = (start..arms_tokens.len())
                    .find(|&j| is_punct(arms_tokens.get(j), ','))
                    .unwrap_or(arms_tokens.len());
                let mut group = Group::new(Delimiter::Brace, stream(&arms_tokens[start..end]));
                if let Some(first) = arms_tokens.get(start) {
                    group.set_span(first.span());
                }
                (group, end)
            }
        };
        arms.push((pattern, body));
        i = end + usize::from(is_punct(arms_tokens.get(end), ','));
    }

    let mut params = Params::default();
    let read = reads(&scrutinee, &mut params);
    let key = scope.capture(params.reader(TokenStream::new(), None, read), &scrutinee);
    let body_tokens: Tokens = arms.iter().flat_map(|(_, body)| body.stream()).collect();
    let arms = arms.iter().map(|(pattern, body)| {
        let bound = bindings(pattern);
        let pattern = stream(pattern);
        let body = rendered(body, scope, bound);
        quote!(#pattern => #body,)
    });
    let render = scope.capture(
        quote! {
            move |__rsn_value: &_| -> ::std::boxed::Box<dyn ::bevy::scene::SceneList> {
                #[allow(unused_variables, clippy::match_single_binding)]
                match ::core::clone::Clone::clone(__rsn_value) { #(#arms)* }
            }
        },
        &body_tokens,
    );
    let switch = private("switch");
    quote!(@#switch(#key, #render))
}

fn for_item(item: &[TokenTree], key: Option<&[TokenTree]>, scope: &mut Scope) -> TokenStream {
    let Some(in_index) = item.iter().position(|t| is_ident(Some(t), "in")) else {
        return error(item[0].span(), "expected `for pattern in items { .. }`");
    };
    let pattern_tokens = &item[1..in_index];
    let pattern = stream(pattern_tokens);
    let Some((iter, body, next)) = until_block(item, in_index + 1) else {
        return error(item[0].span(), "expected `for pattern in items { .. }`");
    };
    if next < item.len() {
        return error(item[next].span(), "unexpected tokens after `for`");
    }
    if let Some((data, filter)) = query_loop(&iter) {
        if key.is_some() {
            return error(
                item[0].span(),
                "a `$Query` loop is keyed by entity; remove `#[key(..)]`",
            );
        }
        return query_for(pattern_tokens, &data, &filter, &body, scope);
    }
    let mut params = Params::default();
    let read = reads(&iter, &mut params);
    let items = scope.capture(
        params.reader(
            TokenStream::new(),
            None,
            quote!(::core::iter::IntoIterator::into_iter(#read).collect::<::std::vec::Vec<_>>()),
        ),
        &iter,
    );
    let bound = bindings(pattern_tokens);
    let body_tokens = tokens_of(&body);
    let rendered = rendered(&body, scope, bound);
    let row = scope.capture(
        quote! {
            move |__rsn_item: &_| -> ::std::boxed::Box<dyn ::bevy::scene::SceneList> {
                #[allow(unused_variables)]
                let #pattern = ::core::clone::Clone::clone(__rsn_item);
                #rendered
            }
        },
        &body_tokens,
    );
    match key {
        Some(key) => {
            if has_dollar(key) {
                return error(item[0].span(), "a `#[key(..)]` can't use `$` reads");
            }
            let key_tokens = stream(key);
            let key = scope.capture(
                quote! {
                    move |__rsn_item: &_| {
                        #[allow(unused_variables)]
                        let #pattern = __rsn_item;
                        #key_tokens
                    }
                },
                key,
            );
            let each = private("each");
            quote!(@#each(#items, #key, #row))
        }
        None => {
            let each = private("each_indexed");
            quote!(@#each(#items, #row))
        }
    }
}

/// `$Query<D, F>` as a whole `for` iterator: returns `D` and `F`.
fn query_loop(iter: &[TokenTree]) -> Option<(TokenStream, TokenStream)> {
    match iter {
        [dollar, TokenTree::Ident(query), lt, generics @ .., gt]
            if is_punct(Some(dollar), '$')
                && query == "Query"
                && is_punct(Some(lt), '<')
                && is_punct(Some(gt), '>') =>
        {
            let parts = split_params(generics);
            match parts.as_slice() {
                [data] => Some((stream(data), quote!(()))),
                [data, filter] => Some((stream(data), stream(filter))),
                _ => None,
            }
        }
        _ => None,
    }
}

/// `for pat in $Query<D, F> { .. }`: one row per matching entity, keyed by
/// entity. A row binds an owned copy of its item, is rebuilt when that
/// changes.
fn query_for(
    pattern: &[TokenTree],
    data: &TokenStream,
    filter: &TokenStream,
    body: &Group,
    scope: &mut Scope,
) -> TokenStream {
    let own = private("Own");
    let items = quote! {
        move |__rsn_query: ::bevy::prelude::Query<(::bevy::prelude::Entity, #data), #filter>| {
            __rsn_query
                .iter()
                .map(|(__rsn_e, __rsn_item)| (__rsn_e, #own::__rsn_own(__rsn_item)))
                .collect::<::std::vec::Vec<_>>()
        }
    };
    let rendered = rendered(body, scope, bindings(pattern));
    let pattern = stream(pattern);
    let row = scope.capture(
        quote! {
            move |__rsn_row: &_| -> ::std::boxed::Box<dyn ::bevy::scene::SceneList> {
                #[allow(unused_variables)]
                let (__rsn_e, #pattern) = ::core::clone::Clone::clone(__rsn_row);
                #rendered
            }
        },
        &tokens_of(body),
    );
    let each = private("each");
    quote!(@#each(#items, |__rsn_row: &(::bevy::prelude::Entity, _)| __rsn_row.0, #row))
}
