//! Procedural macros for `bevy_radioactive`.
//!
//! `coat!` is `bsn!` plus a small Rust subset in scene lists (`if`/`else`,
//! `for` with `#[key(..)]`, `match`, `let`) and `$signal` reads. The macro
//! works on tokens: everything it does not recognise is passed to `bsn!`
//! unchanged.
//!
//! `bsn!` evaluates values inside its own `move` closures, so a signal named
//! directly in two places would be moved twice. Every place that captures a
//! signal (a *site*) therefore reads it through its own clone, declared in
//! the prelude of the enclosing scope: before `bsn!` at the top level, or at
//! the start of a region's render closure.

use proc_macro2::{Delimiter, Group, Ident, Literal, Punct, Spacing, Span, TokenStream, TokenTree};
use quote::{ToTokens, format_ident, quote};

type Tokens = Vec<TokenTree>;

/// BSN with reactive control flow and `$signal` reads.
///
/// See `bevy_radioactive::coat` for the full syntax.
#[proc_macro]
pub fn coat(input: proc_macro::TokenStream) -> proc_macro::TokenStream {
    let tokens: Tokens = TokenStream::from(input).into_iter().collect();
    remember_signals(&tokens);
    let mut scope = Scope::default();
    let body = entity(&tokens, &mut scope);
    let prelude = scope.prelude;
    let bsn = invoke("bsn");
    let output = quote! {{
        #[allow(unused_imports)]
        use ::bevy_radioactive::coat::__private::*;
        #prelude
        #bsn { #body }
    }};
    debug(&output);
    output.into()
}

/// Like `coat!`, producing a scene list (`--` separated entities).
#[proc_macro]
pub fn coat_list(input: proc_macro::TokenStream) -> proc_macro::TokenStream {
    let tokens: Tokens = TokenStream::from(input).into_iter().collect();
    remember_signals(&tokens);
    let mut scope = Scope::default();
    let body = list(&tokens, &mut scope);
    let prelude = scope.prelude;
    let bsn_list = invoke("bsn_list");
    let output = quote! {{
        #[allow(unused_imports)]
        use ::bevy_radioactive::coat::__private::*;
        #prelude
        #bsn_list { #body }
    }};
    debug(&output);
    output.into()
}

fn debug(output: &TokenStream) {
    if std::env::var_os("COAT_DEBUG").is_some() {
        eprintln!("{output}");
    }
}

// ---------------------------------------------------------------------------
// Scopes
// ---------------------------------------------------------------------------

#[derive(Default)]
struct Scope {
    prelude: TokenStream,
    sites: usize,
}

impl Scope {
    /// Declares one clone per signal read in `tokens` (except `bound` names)
    /// and returns the `let name = clone;` bindings for the site.
    fn site(&mut self, tokens: &[TokenTree], bound: &[Ident]) -> TokenStream {
        let mut names = Vec::new();
        signals(tokens, &mut names);
        names.retain(|name| !bound.contains(name));
        let mut bindings = TokenStream::new();
        for name in names {
            let hoisted = format_ident!("__coat_{}_{}", self.sites, name);
            self.sites += 1;
            quote!(let #hoisted = ::core::clone::Clone::clone(&#name);)
                .to_tokens(&mut self.prelude);
            quote!(let #name = ::core::clone::Clone::clone(&#hoisted);).to_tokens(&mut bindings);
        }
        bindings
    }
}

// ---------------------------------------------------------------------------
// Token helpers
// ---------------------------------------------------------------------------

fn is_punct(token: Option<&TokenTree>, ch: char) -> bool {
    matches!(token, Some(TokenTree::Punct(p)) if p.as_char() == ch)
}

fn is_ident(token: Option<&TokenTree>, name: &str) -> bool {
    matches!(token, Some(TokenTree::Ident(i)) if i == name)
}

fn stream(tokens: &[TokenTree]) -> TokenStream {
    tokens.iter().cloned().collect()
}

fn tokens_of(group: &Group) -> Tokens {
    group.stream().into_iter().collect()
}

fn error(span: Span, message: &str) -> TokenStream {
    quote::quote_spanned!(span => ::core::compile_error!(#message);)
}

/// Splits on top-level `--`.
fn split_items(tokens: &[TokenTree]) -> Vec<Tokens> {
    let mut items = vec![Vec::new()];
    let mut i = 0;
    while i < tokens.len() {
        if is_punct(tokens.get(i), '-') && is_punct(tokens.get(i + 1), '-') {
            items.push(Vec::new());
            i += 2;
            continue;
        }
        items.last_mut().unwrap().push(tokens[i].clone());
        i += 1;
    }
    items.retain(|item| !item.is_empty());
    items
}

/// Splits on top-level `,`.
fn split_commas(tokens: &[TokenTree]) -> Vec<Tokens> {
    let mut parts = vec![Vec::new()];
    for token in tokens {
        if is_punct(Some(token), ',') {
            parts.push(Vec::new());
        } else {
            parts.last_mut().unwrap().push(token.clone());
        }
    }
    if parts.last().is_some_and(Vec::is_empty) {
        parts.pop();
    }
    parts
}

fn contains_dollar(tokens: &[TokenTree]) -> bool {
    tokens.iter().any(|token| match token {
        TokenTree::Punct(p) => p.as_char() == '$',
        TokenTree::Group(g) => contains_dollar(&tokens_of(g)),
        _ => false,
    })
}

/// Collects every signal used, deeply: `$name` reads, and bare uses of a name
/// that is read with `$` somewhere in the macro (such as `count.set(0)`).
fn signals(tokens: &[TokenTree], out: &mut Vec<Ident>) {
    for (i, token) in tokens.iter().enumerate() {
        match token {
            TokenTree::Ident(name) if is_signal(name) && !out.contains(name) => {
                out.push(name.clone());
            }
            TokenTree::Punct(p) if p.as_char() == '$' => {
                if let Some(TokenTree::Ident(name)) = tokens.get(i + 1)
                    && !out.contains(name)
                {
                    out.push(name.clone());
                }
            }
            TokenTree::Group(g) => signals(&tokens_of(g), out),
            _ => {}
        }
    }
}

/// Identifiers bound by a pattern (approximate: every lowercase identifier).
fn bindings(pattern: &[TokenTree]) -> Vec<Ident> {
    let mut out = Vec::new();
    for token in pattern {
        match token {
            TokenTree::Ident(ident) if ident.to_string().starts_with(char::is_lowercase) => {
                out.push(ident.clone());
            }
            TokenTree::Group(g) => out.extend(bindings(&tokens_of(g))),
            _ => {}
        }
    }
    out
}

/// Rewrites `$name` to `name.get()` and `$(expr)` to `(expr)`, deeply.
fn reads(tokens: &[TokenTree]) -> TokenStream {
    let mut out = TokenStream::new();
    let mut i = 0;
    while i < tokens.len() {
        match &tokens[i] {
            TokenTree::Punct(p) if p.as_char() == '$' => match tokens.get(i + 1) {
                Some(TokenTree::Ident(name)) => {
                    quote!(#name.get()).to_tokens(&mut out);
                    i += 2;
                    continue;
                }
                Some(TokenTree::Group(g)) if g.delimiter() == Delimiter::Parenthesis => {
                    Group::new(Delimiter::Parenthesis, reads(&tokens_of(g))).to_tokens(&mut out);
                    i += 2;
                    continue;
                }
                _ => error(p.span(), "expected `$name` or `$(expr)`").to_tokens(&mut out),
            },
            TokenTree::Group(g) => {
                let mut group = Group::new(g.delimiter(), reads(&tokens_of(g)));
                group.set_span(g.span());
                group.to_tokens(&mut out);
            }
            other => other.to_tokens(&mut out),
        }
        i += 1;
    }
    out
}

/// An expression that reads its signals through its own clones.
fn capturing(tokens: &[TokenTree], scope: &mut Scope) -> TokenStream {
    if !mentions_signal(tokens) {
        return stream(tokens);
    }
    let site = scope.site(tokens, &[]);
    let expr = reads(tokens);
    quote! {{ #site #expr }}
}

// ---------------------------------------------------------------------------
// Entities
// ---------------------------------------------------------------------------

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
                TokenTree::Group(g) if mentions_signal(&tokens_of(g)) => {
                    // `{ expr }` values are evaluated once, while spawning.
                    let mut group = Group::new(g.delimiter(), capturing(&tokens_of(g), scope));
                    group.set_span(g.span());
                    group.to_tokens(&mut out);
                }
                other => other.to_tokens(&mut out),
            }
            i += 1;
            continue;
        }

        let start = i;
        let mut last = None;
        if is_punct(tokens.get(i), ':') {
            i += 2;
        }
        while let Some(TokenTree::Ident(ident)) = tokens.get(i) {
            last = Some(ident.clone());
            i += 1;
            if is_punct(tokens.get(i), ':') && is_punct(tokens.get(i + 1), ':') {
                i += 2;
            } else {
                break;
            }
        }
        let path = &tokens[start..i];
        let is_component = last
            .as_ref()
            .is_some_and(|ident| ident.to_string().starts_with(char::is_uppercase));

        match tokens.get(i) {
            Some(TokenTree::Group(g)) if g.delimiter() == Delimiter::Bracket => {
                stream(path).to_tokens(&mut out);
                Group::new(Delimiter::Bracket, list(&tokens_of(g), scope)).to_tokens(&mut out);
                i += 1;
            }
            Some(TokenTree::Group(g))
                if (is_component && contains_dollar(&tokens_of(g)))
                    || (!is_component && mentions_signal(&tokens_of(g))) =>
            {
                let content = tokens_of(g);
                if is_component {
                    reactive_component(path, g.delimiter(), &content, scope).to_tokens(&mut out);
                } else {
                    // A function call such as `on(...)`: each argument reads
                    // its signals through its own clones.
                    let args = split_commas(&content)
                        .iter()
                        .map(|arg| capturing(arg, scope))
                        .collect::<Vec<_>>();
                    stream(path).to_tokens(&mut out);
                    Group::new(g.delimiter(), quote!(#(#args),*)).to_tokens(&mut out);
                }
                i += 1;
            }
            _ => stream(path).to_tokens(&mut out),
        }
    }
    out
}

/// `Comp($x)` → `Comp({ x.get() }) __coat_patch(move |c: &mut Comp| c.0 = ..)`.
fn reactive_component(
    path: &[TokenTree],
    delimiter: Delimiter,
    content: &[TokenTree],
    scope: &mut Scope,
) -> TokenStream {
    let fields = split_commas(content);
    let mut initial = Vec::new();
    let mut assignments = TokenStream::new();
    for (index, field) in fields.iter().enumerate() {
        if !contains_dollar(field) {
            initial.push(stream(field));
            continue;
        }
        let (name, value) = if delimiter == Delimiter::Parenthesis {
            (None, &field[..])
        } else {
            let colon = (0..field.len()).find(|&j| {
                is_punct(field.get(j), ':')
                    && !is_punct(field.get(j + 1), ':')
                    && !(j > 0 && is_punct(field.get(j - 1), ':'))
            });
            match (colon, field.first()) {
                (Some(colon), Some(TokenTree::Ident(name))) => {
                    (Some(name.clone()), &field[colon + 1..])
                }
                _ => return error(path[0].span(), "expected `field: value` with `$`"),
            }
        };
        let read = reads(value);
        let site = scope.site(value, &[]);
        match &name {
            Some(name) => {
                initial.push(quote!(#name: {{ #site #read }}));
                quote!(__c.#name = ::core::convert::Into::into(#read);).to_tokens(&mut assignments);
            }
            None => {
                initial.push(quote!({{ #site #read }}));
                let index = Literal::usize_unsuffixed(index);
                quote!(__c.#index = ::core::convert::Into::into(#read);)
                    .to_tokens(&mut assignments);
            }
        }
    }
    let path = stream(path);
    let initial = Group::new(delimiter, quote!(#(#initial),*));
    let site = scope.site(content, &[]);
    quote! {
        #path #initial
        __coat_patch({ #site move |__c: &mut #path| { #assignments } })
    }
}

// ---------------------------------------------------------------------------
// Scene lists and control flow
// ---------------------------------------------------------------------------

/// Transforms the `--` separated items of a relationship list.
fn list(tokens: &[TokenTree], scope: &mut Scope) -> TokenStream {
    let items = split_items(tokens);
    let mut out: Vec<TokenStream> = Vec::new();
    for (index, item) in items.iter().enumerate() {
        if is_ident(item.first(), "let") {
            out.push(let_item(item, &items[index + 1..], scope));
            break;
        }
        out.push(list_item(item, scope));
    }
    let mut joined = TokenStream::new();
    for (index, item) in out.into_iter().enumerate() {
        if index > 0 {
            dashes().to_tokens(&mut joined);
        }
        item.to_tokens(&mut joined);
    }
    joined
}

fn dashes() -> TokenStream {
    [
        TokenTree::Punct(Punct::new('-', Spacing::Joint)),
        TokenTree::Punct(Punct::new('-', Spacing::Alone)),
    ]
    .into_iter()
    .collect()
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
        error(item[0].span(), "`#[key(...)]` must precede a `for` loop")
    } else if is_ident(rest.first(), "if") {
        if_item(rest, scope)
    } else if is_ident(rest.first(), "match") {
        match_item(rest, scope)
    } else {
        entity(rest, scope)
    }
}

/// `let x = expr; rest...` → `{ let x = expr; bsn_list! { rest } }`.
fn let_item(item: &[TokenTree], following: &[Tokens], scope: &mut Scope) -> TokenStream {
    let mut statements = TokenStream::new();
    let mut i = 0;
    while is_ident(item.get(i), "let") {
        let end = (i..item.len())
            .find(|&j| is_punct(item.get(j), ';'))
            .unwrap_or(item.len());
        capturing_statement(&item[i..end], scope).to_tokens(&mut statements);
        i = end + 1;
    }
    let mut rest: Tokens = item[i.min(item.len())..].to_vec();
    for next in following {
        if !rest.is_empty() {
            rest.extend(dashes());
        }
        rest.extend(next.iter().cloned());
    }
    let body = list(&rest, scope);
    let bsn_list = invoke("bsn_list");
    // Doubled braces: `bsn!` unwraps one level of a list item expression.
    quote! {{ { #statements #bsn_list { #body } } }}
}

fn capturing_statement(statement: &[TokenTree], scope: &mut Scope) -> TokenStream {
    let Some(eq) = statement.iter().position(|t| is_punct(Some(t), '=')) else {
        return error(statement[0].span(), "expected `let pattern = value;`");
    };
    let binding = stream(&statement[..eq]);
    let value = capturing(&statement[eq + 1..], scope);
    quote!(#binding = #value;)
}

/// Splits at the first top-level brace group (a Rust condition or
/// scrutinee): returns the tokens before it, the group, and the next index.
fn until_block(tokens: &[TokenTree], from: usize) -> Option<(Tokens, Group, usize)> {
    let index = (from..tokens.len()).find(
        |&j| matches!(&tokens[j], TokenTree::Group(g) if g.delimiter() == Delimiter::Brace),
    )?;
    let TokenTree::Group(group) = &tokens[index] else {
        unreachable!()
    };
    Some((tokens[from..index].to_vec(), group.clone(), index + 1))
}

/// A body evaluated once, in the current scope.
fn boxed_list(body: &Group, scope: &mut Scope) -> TokenStream {
    let body = list(&tokens_of(body), scope);
    let bsn_list = invoke("bsn_list");
    quote! {
        ::std::boxed::Box::new(#bsn_list { #body })
            as ::std::boxed::Box<dyn ::bevy::scene::SceneList>
    }
}

/// A body evaluated on every render, with its own scope.
fn rendered_list(body: &Group) -> TokenStream {
    let mut scope = Scope::default();
    let list = boxed_list(body, &mut scope);
    let prelude = scope.prelude;
    quote!({ #prelude #list })
}

fn if_item(item: &[TokenTree], scope: &mut Scope) -> TokenStream {
    let mut branches: Vec<(Option<Tokens>, Group)> = Vec::new();
    let mut i = 0;
    loop {
        let Some((cond, body, next)) = until_block(item, i + 1) else {
            return error(item[i].span(), "expected `if condition { ... }`");
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
            _ => return error(item[i].span(), "expected `else { ... }`"),
        }
    }
    if i < item.len() {
        return error(item[i].span(), "unexpected tokens after `if`");
    }
    let has_else = branches.last().is_some_and(|(cond, _)| cond.is_none());
    let reactive = branches
        .iter()
        .any(|(cond, _)| cond.as_ref().is_some_and(|c| contains_dollar(c)));

    if !reactive {
        let mut chain = TokenStream::new();
        for (index, (cond, body)) in branches.iter().enumerate() {
            let body = boxed_list(body, scope);
            if index > 0 {
                quote!(else).to_tokens(&mut chain);
            }
            match cond {
                Some(cond) => {
                    let cond = stream(cond);
                    quote!(if #cond { ::core::option::Option::Some(#body) }).to_tokens(&mut chain);
                }
                None => quote!({ ::core::option::Option::Some(#body) }).to_tokens(&mut chain),
            }
        }
        if !has_else {
            quote!(else { ::core::option::Option::None }).to_tokens(&mut chain);
        }
        return quote!({ #chain });
    }

    let mut selector = TokenStream::new();
    let mut cond_tokens = Tokens::new();
    for (index, (cond, _)) in branches.iter().enumerate() {
        let index = Literal::usize_unsuffixed(index);
        if index.to_string() != "0" {
            quote!(else).to_tokens(&mut selector);
        }
        match cond {
            Some(cond) => {
                cond_tokens.extend(cond.iter().cloned());
                let cond = reads(cond);
                quote!(if #cond { ::core::option::Option::Some(#index) }).to_tokens(&mut selector);
            }
            None => quote!({ ::core::option::Option::Some(#index) }).to_tokens(&mut selector),
        }
    }
    if !has_else {
        quote!(else { ::core::option::Option::None }).to_tokens(&mut selector);
    }
    let cond_site = scope.site(&cond_tokens, &[]);

    let body_tokens: Tokens = branches
        .iter()
        .flat_map(|(_, body)| body.stream())
        .collect();
    let body_site = scope.site(&body_tokens, &[]);
    let arms = branches.iter().enumerate().map(|(index, (_, body))| {
        let index = Literal::usize_unsuffixed(index);
        let body = rendered_list(body);
        quote!(::core::option::Option::Some(#index) => ::core::option::Option::Some(#body),)
    });
    quote! {
        __CoatRegion
        __coat_region(
            { #cond_site move || -> ::core::option::Option<usize> { #selector } },
            { #body_site move |__branch: &::core::option::Option<usize>| match *__branch {
                #(#arms)*
                _ => ::core::option::Option::None,
            } }
        )
    }
}

fn for_item(item: &[TokenTree], key: Option<&[TokenTree]>, scope: &mut Scope) -> TokenStream {
    let Some(in_index) = item.iter().position(|t| is_ident(Some(t), "in")) else {
        return error(item[0].span(), "expected `for pattern in items { ... }`");
    };
    let pattern_tokens = &item[1..in_index];
    let pattern = stream(pattern_tokens);
    let Some((iter, body, next)) = until_block(item, in_index + 1) else {
        return error(item[0].span(), "expected `for pattern in items { ... }`");
    };
    if next < item.len() {
        return error(item[next].span(), "unexpected tokens after `for`");
    }

    if !contains_dollar(&iter) {
        let iter = stream(&iter);
        let body = boxed_list(&body, scope);
        return quote! {{
            ::core::iter::IntoIterator::into_iter(#iter)
                .map(|#pattern| #body)
                .collect::<::std::vec::Vec<_>>()
        }};
    }

    let bound = bindings(pattern_tokens);
    let iter_site = scope.site(&iter, &[]);
    let iter = reads(&iter);
    let body_site = scope.site(&tokens_of(&body), &bound);
    let body = rendered_list(&body);
    let items = quote! {
        { #iter_site move || ::core::iter::IntoIterator::into_iter(#iter).collect::<::std::vec::Vec<_>>() }
    };
    let render = quote! {
        { #body_site move |__item: &_| {
            #[allow(unused_variables)]
            let #pattern = ::core::clone::Clone::clone(__item);
            #body
        } }
    };
    match key {
        Some(key) => {
            let key_site = scope.site(key, &bound);
            let key = reads(key);
            quote! {
                __CoatRegion
                __coat_for(
                    #items,
                    { #key_site move |__item: &_| {
                        #[allow(unused_variables)]
                        let #pattern = __item;
                        #key
                    } },
                    #render
                )
            }
        }
        None => quote! {
            __CoatRegion
            __coat_for_indexed(#items, #render)
        },
    }
}

fn match_item(item: &[TokenTree], scope: &mut Scope) -> TokenStream {
    let Some((scrutinee, arms_group, next)) = until_block(item, 1) else {
        return error(item[0].span(), "expected `match value { ... }`");
    };
    if next < item.len() {
        return error(item[next].span(), "unexpected tokens after `match`");
    }
    let arms_tokens = tokens_of(&arms_group);

    // `pattern => body,` where a body is `{ ... }` or tokens up to `,`.
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
            Some(TokenTree::Group(g))
                if g.delimiter() == Delimiter::Brace
                    && (start + 1 == arms_tokens.len()
                        || is_punct(arms_tokens.get(start + 1), ',')) =>
            {
                (g.clone(), start + 1)
            }
            _ => {
                let end = (start..arms_tokens.len())
                    .find(|&j| is_punct(arms_tokens.get(j), ','))
                    .unwrap_or(arms_tokens.len());
                (
                    Group::new(Delimiter::Brace, stream(&arms_tokens[start..end])),
                    end,
                )
            }
        };
        arms.push((pattern, body));
        i = end + usize::from(is_punct(arms_tokens.get(end), ','));
    }

    if !contains_dollar(&scrutinee) {
        let scrutinee = stream(&scrutinee);
        let arms = arms
            .iter()
            .map(|(pattern, body)| {
                let pattern = stream(pattern);
                let body = boxed_list(body, scope);
                quote!(#pattern => #body,)
            })
            .collect::<Vec<_>>();
        return quote!({ match #scrutinee { #(#arms)* } });
    }

    let scrutinee_site = scope.site(&scrutinee, &[]);
    let scrutinee = reads(&scrutinee);
    let bound: Vec<Ident> = arms
        .iter()
        .flat_map(|(pattern, _)| bindings(pattern))
        .collect();
    let body_tokens: Tokens = arms.iter().flat_map(|(_, body)| body.stream()).collect();
    let body_site = scope.site(&body_tokens, &bound);
    let arms = arms.iter().map(|(pattern, body)| {
        let pattern = stream(pattern);
        let body = rendered_list(body);
        quote!(#pattern => ::core::option::Option::Some(#body),)
    });
    quote! {
        __CoatRegion
        __coat_region(
            { #scrutinee_site move || #scrutinee },
            { #body_site move |__value: &_| {
                #[allow(unused_variables, clippy::match_single_binding)]
                match ::core::clone::Clone::clone(__value) { #(#arms)* }
            } }
        )
    }
}

/// Invokes `bsn!`/`bsn_list!` through `macro_rules!` wrappers, so the call
/// site `bsn!` inspects is its own name rather than `coat!`.
fn invoke(name: &str) -> TokenStream {
    let name = Ident::new(&format!("__coat_{name}"), Span::call_site());
    quote!(::bevy_radioactive::#name!)
}

// ---------------------------------------------------------------------------
// #[component]
// ---------------------------------------------------------------------------

/// Turns `fn name(cx: &Cx, props..) -> impl Scene` into a component that is
/// used in `bsn!` like a scene function: `@name(props..)`.
///
/// See `bevy_radioactive::webby` for details.
#[proc_macro_attribute]
pub fn component(
    attr: proc_macro::TokenStream,
    item: proc_macro::TokenStream,
) -> proc_macro::TokenStream {
    if !attr.is_empty() {
        return error(Span::call_site(), "`#[component]` takes no arguments").into();
    }
    match syn::parse::<syn::ItemFn>(item) {
        Ok(function) => component_fn(function)
            .unwrap_or_else(syn::Error::into_compile_error)
            .into(),
        Err(error) => error.into_compile_error().into(),
    }
}

fn last_segment_is(ty: &syn::Type, name: &str) -> bool {
    match ty {
        syn::Type::Path(path) => path.path.segments.last().is_some_and(|s| s.ident == name),
        syn::Type::Reference(reference) => last_segment_is(&reference.elem, name),
        _ => false,
    }
}

fn component_fn(function: syn::ItemFn) -> syn::Result<TokenStream> {
    let syn::ItemFn {
        attrs,
        vis,
        sig,
        block,
    } = function;
    if let Some(asyncness) = &sig.asyncness {
        return Err(syn::Error::new_spanned(
            asyncness,
            "components cannot be async",
        ));
    }
    let webby = quote!(::bevy_radioactive::webby);
    let entity = quote!(::bevy::prelude::Entity);

    let mut uses_cx = false;
    let mut outer_inputs = Vec::new();
    let mut captured = Vec::new();
    let mut entities = Vec::new();
    let mut call_args = Vec::new();
    for (index, input) in sig.inputs.iter().enumerate() {
        let syn::FnArg::Typed(typed) = input else {
            return Err(syn::Error::new_spanned(
                input,
                "components cannot take `self`",
            ));
        };
        let syn::Pat::Ident(pat) = &*typed.pat else {
            return Err(syn::Error::new_spanned(
                &typed.pat,
                "component props must be plain names",
            ));
        };
        let name = &pat.ident;
        let ty = &typed.ty;
        if index == 0 && matches!(&**ty, syn::Type::Reference(_)) && last_segment_is(ty, "Cx") {
            uses_cx = true;
            call_args.push(quote!(__cx));
        } else if last_segment_is(ty, "Entity") && !matches!(&**ty, syn::Type::Reference(_)) {
            // `#Name` in `bsn!` passes an `EntityTemplate`; resolve it as a dep.
            outer_inputs.push(quote!(#name: impl #webby::Deps<Output = #entity>));
            entities.push(name.clone());
            call_args.push(quote!(#name));
        } else {
            outer_inputs.push(quote!(#name: #ty));
            captured.push(name.clone());
            call_args.push(quote!(::core::clone::Clone::clone(#name)));
        }
    }
    let _ = uses_cx;

    // The render function must not borrow from `cx`.
    let mut render_sig = sig.clone();
    render_sig.ident = Ident::new("__render", Span::call_site());
    if let syn::ReturnType::Type(_, ty) = &mut render_sig.output
        && let syn::Type::ImplTrait(impl_trait) = &mut **ty
        && !impl_trait
            .bounds
            .iter()
            .any(|bound| matches!(bound, syn::TypeParamBound::PreciseCapture(_)))
    {
        let params = sig.generics.type_params().map(|param| &param.ident);
        let bound: syn::TypeParamBound = syn::parse_quote!(use<#(#params),*>);
        impl_trait.bounds.push(bound);
    }

    let name = &sig.ident;
    let (impl_generics, _, where_clause) = sig.generics.split_for_impl();
    let turbofish = sig
        .generics
        .type_params()
        .map(|param| &param.ident)
        .collect::<Vec<_>>();
    let turbofish = if turbofish.is_empty() {
        quote!()
    } else {
        quote!(::<#(#turbofish),*>)
    };
    let entity_types = entities.iter().map(|_| entity.clone()).collect::<Vec<_>>();

    Ok(quote! {
        #(#attrs)*
        #vis fn #name #impl_generics (#(#outer_inputs),*)
            -> #webby::Mount<
                impl #webby::Deps<Output = (#(#entity_types,)*)>,
                impl #webby::Render<(#(#entity_types,)*)>,
            >
        #where_clause
        {
            #[allow(clippy::needless_pass_by_value)]
            #render_sig #block

            let __props = (#(#captured,)*);
            #webby::component_with(
                (#(#entities,)*),
                move |__cx: &#webby::Cx, (#(#entities,)*): (#(#entity_types,)*)| {
                    #[allow(unused_variables)]
                    let (#(#captured,)*) = &__props;
                    __render #turbofish (#(#call_args),*)
                },
            )
        }
    })
}

// ---------------------------------------------------------------------------
// Known signals
// ---------------------------------------------------------------------------

thread_local! {
    /// Names read with `$` anywhere in the current `coat!` invocation.
    static SIGNALS: std::cell::RefCell<Vec<String>> = const { std::cell::RefCell::new(Vec::new()) };
}

fn remember_signals(tokens: &[TokenTree]) {
    fn dollar_names(tokens: &[TokenTree], out: &mut Vec<String>) {
        for (i, token) in tokens.iter().enumerate() {
            match token {
                TokenTree::Punct(p) if p.as_char() == '$' => {
                    if let Some(TokenTree::Ident(name)) = tokens.get(i + 1) {
                        out.push(name.to_string());
                    }
                }
                TokenTree::Group(g) => dollar_names(&tokens_of(g), out),
                _ => {}
            }
        }
    }
    let mut names = Vec::new();
    dollar_names(tokens, &mut names);
    SIGNALS.with(|signals| *signals.borrow_mut() = names);
}

fn is_signal(name: &Ident) -> bool {
    SIGNALS.with(|signals| signals.borrow().iter().any(|signal| name == signal))
}

/// Whether `tokens` read a signal with `$` or use a known signal by name.
fn mentions_signal(tokens: &[TokenTree]) -> bool {
    tokens.iter().any(|token| match token {
        TokenTree::Punct(p) => p.as_char() == '$',
        TokenTree::Ident(name) => is_signal(name),
        TokenTree::Group(g) => mentions_signal(&tokens_of(g)),
        _ => false,
    })
}
