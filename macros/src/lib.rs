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
                if let Some(name) = dollar_name(tokens, i)
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

/// The signal named by the `$` at `index`: `$name` or `$&name`.
fn dollar_name(tokens: &[TokenTree], index: usize) -> Option<&Ident> {
    let next = if is_punct(tokens.get(index + 1), '&') {
        index + 2
    } else {
        index + 1
    };
    match tokens.get(next) {
        Some(TokenTree::Ident(name)) => Some(name),
        _ => None,
    }
}

/// End of the method-call and index chain starting at `from`, such as
/// `.len()` or `.items[0].name`.
fn postfix_end(tokens: &[TokenTree], from: usize) -> usize {
    let mut i = from;
    loop {
        if is_punct(tokens.get(i), '.') && matches!(tokens.get(i + 1), Some(TokenTree::Ident(_))) {
            i += 2;
            if matches!(tokens.get(i), Some(TokenTree::Group(g)) if g.delimiter() == Delimiter::Parenthesis)
            {
                i += 1;
            }
        } else if matches!(tokens.get(i), Some(TokenTree::Group(g)) if g.delimiter() == Delimiter::Bracket)
        {
            i += 1;
        } else {
            return i;
        }
    }
}

/// Rewrites `$name` to `name.get()`, `$&name.chain` to
/// `name.with(|name| name.chain)`, and `$(expr)` to `(expr)`, deeply.
fn reads(tokens: &[TokenTree]) -> TokenStream {
    let mut out = TokenStream::new();
    let mut i = 0;
    while i < tokens.len() {
        match &tokens[i] {
            TokenTree::Punct(p) if p.as_char() == '$' => match tokens.get(i + 1) {
                Some(TokenTree::Punct(amp)) if amp.as_char() == '&' => {
                    let Some(TokenTree::Ident(name)) = tokens.get(i + 2) else {
                        error(p.span(), "expected `$&name`").to_tokens(&mut out);
                        i += 2;
                        continue;
                    };
                    let end = postfix_end(tokens, i + 3);
                    let chain = reads(&tokens[i + 3..end]);
                    quote!(#name.with(|#name| #name #chain)).to_tokens(&mut out);
                    i = end;
                    continue;
                }
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
                _ => error(p.span(), "expected `$name`, `$&name` or `$(expr)`").to_tokens(&mut out),
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

/// `Comp($x)` → `Comp({ x.get() }) __coat_patch(move |c: &mut Comp| ..)`,
/// where the patch assigns only fields whose value changed.
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
                quote!(__changed |= __coat_assign(&mut __c.#name, ::core::convert::Into::into(#read));)
                    .to_tokens(&mut assignments);
            }
            None => {
                initial.push(quote!({{ #site #read }}));
                let index = Literal::usize_unsuffixed(index);
                quote!(__changed |= __coat_assign(&mut __c.#index, ::core::convert::Into::into(#read));)
                    .to_tokens(&mut assignments);
            }
        }
    }
    let path = stream(path);
    let initial = Group::new(delimiter, quote!(#(#initial),*));
    let site = scope.site(content, &[]);
    quote! {
        #path #initial
        __coat_patch({ #site move |__c: &mut #path| -> bool {
            let mut __changed = false;
            #assignments
            __changed
        } })
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
    let mut pattern_tokens = &item[1..in_index];
    // `for $item in ..`: each row receives its item as a signal.
    let item_signal = is_punct(pattern_tokens.first(), '$');
    if item_signal {
        if !matches!(pattern_tokens, [_, TokenTree::Ident(_)]) {
            return error(
                pattern_tokens[0].span(),
                "expected `for $name in ..`: an item signal binds a single name",
            );
        }
        pattern_tokens = &pattern_tokens[1..];
    }
    let pattern = stream(pattern_tokens);
    let Some((iter, body, next)) = until_block(item, in_index + 1) else {
        return error(item[0].span(), "expected `for pattern in items { ... }`");
    };
    if next < item.len() {
        return error(item[next].span(), "unexpected tokens after `for`");
    }
    if item_signal && !contains_dollar(&iter) {
        return error(
            item[1].span(),
            "`for $name in ..` needs a reactive iterator that reads a signal with `$`",
        );
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
    let (keyed, indexed) = if item_signal {
        (quote!(__coat_for_signal), quote!(__coat_for_indexed_signal))
    } else {
        (quote!(__coat_for), quote!(__coat_for_indexed))
    };
    match key {
        Some(key) => {
            let key_site = scope.site(key, &bound);
            let key = reads(key);
            quote! {
                __CoatRegion
                #keyed(
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
            #indexed(#items, #render)
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

    // The region is keyed by the matched arm plus the bound values the arm
    // uses, so a change elsewhere in the scrutinee does not rebuild the arm.
    let key_site = scope.site(&scrutinee, &[]);
    let scrutinee_reads = reads(&scrutinee);
    let bound: Vec<Ident> = arms
        .iter()
        .flat_map(|(pattern, _)| pattern_bindings(pattern))
        .collect();
    let body_tokens: Tokens = arms.iter().flat_map(|(_, body)| body.stream()).collect();
    // Signals the arms capture are already in scope for the scrutinee.
    let mut shared = Vec::new();
    signals(&body_tokens, &mut shared);
    shared.retain(|name| !bound.contains(name));
    let render_site = scope.site(&scrutinee, &shared);
    let body_site = scope.site(&body_tokens, &bound);
    let keys = arms.iter().enumerate().map(|(index, (pattern, body))| {
        let body = tokens_of(body);
        let used: Vec<Ident> = pattern_bindings(pattern)
            .into_iter()
            .filter(|name| mentions(&body, name))
            .collect();
        let pattern = stream(pattern);
        let index = Literal::usize_unsuffixed(index);
        quote!(#pattern => __coat_arm_key(#index, (#(#used,)*)),)
    });
    let arms = arms.iter().map(|(pattern, body)| {
        let pattern = stream(pattern);
        let body = rendered_list(body);
        quote!(#pattern => ::core::option::Option::Some(#body),)
    });
    quote! {
        __CoatRegion
        __coat_region(
            { #key_site move || {
                #[allow(unused_variables, clippy::match_single_binding)]
                match #scrutinee_reads { #(#keys)* }
            } },
            { #render_site #body_site move |_: &__CoatArmKey| {
                #[allow(unused_variables, clippy::match_single_binding)]
                match #scrutinee_reads { #(#arms)* }
            } }
        )
    }
}

/// Identifiers bound by a `match` pattern: lowercase names that are not path
/// segments, field names, keywords or part of a guard.
fn pattern_bindings(pattern: &[TokenTree]) -> Vec<Ident> {
    let guard = pattern
        .iter()
        .position(|token| is_ident(Some(token), "if"))
        .unwrap_or(pattern.len());
    let pattern = &pattern[..guard];
    let mut out: Vec<Ident> = Vec::new();
    for (i, token) in pattern.iter().enumerate() {
        match token {
            TokenTree::Ident(ident) => {
                let name = ident.to_string();
                let path_segment = (is_punct(pattern.get(i + 1), ':')
                    && is_punct(pattern.get(i + 2), ':'))
                    || (i >= 2
                        && is_punct(pattern.get(i - 1), ':')
                        && is_punct(pattern.get(i - 2), ':'));
                let field_name = is_punct(pattern.get(i + 1), ':') && !path_segment;
                let keyword = matches!(name.as_str(), "ref" | "mut" | "box" | "true" | "false");
                if name.starts_with(char::is_lowercase)
                    && !path_segment
                    && !field_name
                    && !keyword
                    && !out.contains(ident)
                {
                    out.push(ident.clone());
                }
            }
            TokenTree::Group(g) => {
                for ident in pattern_bindings(&tokens_of(g)) {
                    if !out.contains(&ident) {
                        out.push(ident);
                    }
                }
            }
            _ => {}
        }
    }
    out
}

/// Whether `tokens` mention the identifier `name`, deeply.
fn mentions(tokens: &[TokenTree], name: &Ident) -> bool {
    tokens.iter().any(|token| match token {
        TokenTree::Ident(ident) => ident == name,
        TokenTree::Group(g) => mentions(&tokens_of(g), name),
        _ => false,
    })
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

/// Whether a prop is marked `#[entity]`, removing the attribute.
fn take_entity_attr(attrs: &mut Vec<syn::Attribute>) -> syn::Result<bool> {
    let mut found = false;
    let mut result = Ok(());
    attrs.retain(|attr| {
        if !attr.path().is_ident("entity") {
            return true;
        }
        if !matches!(attr.meta, syn::Meta::Path(_)) {
            result = Err(syn::Error::new_spanned(
                attr,
                "`#[entity]` takes no arguments",
            ));
        }
        found = true;
        false
    });
    result.map(|()| found)
}

/// Adds `use<..>` to every `impl Trait` in a return type that lacks one, so
/// the render function never captures the `cx` lifetime.
fn add_precise_capture(ty: &mut syn::Type, generics: &syn::Generics) {
    match ty {
        syn::Type::ImplTrait(impl_trait) => {
            if !impl_trait
                .bounds
                .iter()
                .any(|bound| matches!(bound, syn::TypeParamBound::PreciseCapture(_)))
            {
                let params = generics.type_params().map(|param| &param.ident);
                let bound: syn::TypeParamBound = syn::parse_quote!(use<#(#params),*>);
                impl_trait.bounds.push(bound);
            }
        }
        syn::Type::Path(path) => {
            for segment in &mut path.path.segments {
                if let syn::PathArguments::AngleBracketed(args) = &mut segment.arguments {
                    for arg in &mut args.args {
                        if let syn::GenericArgument::Type(ty) = arg {
                            add_precise_capture(ty, generics);
                        }
                    }
                }
            }
        }
        syn::Type::Paren(paren) => add_precise_capture(&mut paren.elem, generics),
        syn::Type::Group(group) => add_precise_capture(&mut group.elem, generics),
        _ => {}
    }
}

fn component_fn(function: syn::ItemFn) -> syn::Result<TokenStream> {
    let syn::ItemFn {
        attrs,
        vis,
        mut sig,
        block,
    } = function;
    if let Some(asyncness) = &sig.asyncness {
        return Err(syn::Error::new_spanned(
            asyncness,
            "components cannot be async",
        ));
    }
    let webby = quote!(::bevy_radioactive::webby);

    let mut outer_inputs = Vec::new();
    let mut captured = Vec::new();
    let mut entities = Vec::new();
    let mut entity_types = Vec::new();
    let mut call_args = Vec::new();
    for (index, input) in sig.inputs.iter_mut().enumerate() {
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
        let marked = take_entity_attr(&mut typed.attrs)?;
        let ty = &typed.ty;
        let reference = matches!(&**ty, syn::Type::Reference(_));
        if marked && reference {
            return Err(syn::Error::new_spanned(
                ty,
                "`#[entity]` props are passed by value",
            ));
        }
        if index == 0 && reference && last_segment_is(ty, "Cx") {
            call_args.push(quote!(__cx));
        } else if marked || (last_segment_is(ty, "Entity") && !reference) {
            // `#Name` in `bsn!` passes an `EntityTemplate`; resolve it as a dep.
            outer_inputs.push(quote!(#name: impl #webby::Deps<Output = #ty>));
            entities.push(name.clone());
            entity_types.push(quote!(#ty));
            call_args.push(quote!(#name));
        } else {
            outer_inputs.push(quote!(#name: #ty));
            captured.push(name.clone());
            call_args.push(quote!(::core::clone::Clone::clone(#name)));
        }
    }

    // The render function must not borrow from `cx`.
    let mut render_sig = sig.clone();
    render_sig.ident = Ident::new("__render", Span::call_site());
    let mut fallible = false;
    if let syn::ReturnType::Type(_, ty) = &mut render_sig.output {
        fallible = last_segment_is(ty, "Result") && !matches!(&**ty, syn::Type::Reference(_));
        add_precise_capture(ty, &sig.generics);
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
    let render_call = quote!(__render #turbofish (#(#call_args),*));
    let render_call = if fallible {
        // Errors go to Bevy's error handler; the component renders nothing.
        quote!(#webby::__render_result(__cx, ::core::stringify!(#name), #render_call))
    } else {
        render_call
    };

    Ok(quote! {
        #(#attrs)*
        #[track_caller]
        #vis fn #name #impl_generics (#(#outer_inputs),*)
            -> #webby::Mount<
                impl #webby::Deps<Output = (#(#entity_types,)*)>,
                impl #webby::Render<(#(#entity_types,)*)>,
            >
        #where_clause
        {
            #[allow(clippy::needless_pass_by_value)]
            #render_sig #block

            let __props = ::std::sync::Arc::new((#(#captured,)*));
            let __memo = ::std::sync::Arc::clone(&__props);
            #webby::component_with(
                (#(#entities,)*),
                move |__cx: &#webby::Cx, (#(#entities,)*): (#(#entity_types,)*)| {
                    #[allow(unused_variables)]
                    let (#(#captured,)*) = &*__props;
                    #render_call
                },
            )
            // A parent re-render skips this component when its props are
            // unchanged, if they are all `PartialEq`.
            .__memo(
                __memo,
                |__old, __new| {
                    #[allow(unused_imports)]
                    use #webby::{__ProbeEq as _, __ProbeOpaque as _};
                    (&#webby::__Probe(__old, __new)).__props_eq()
                },
                |__old, __new| {
                    #[allow(unused_imports)]
                    use #webby::{__ProbeEq as _, __ProbeOpaque as _};
                    (&#webby::__Probe(__old, __new)).__props_eq()
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
                    if let Some(name) = dollar_name(tokens, i) {
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

// ---------------------------------------------------------------------------
// #[derive(Store)]
// ---------------------------------------------------------------------------

/// Derives `bevy_radioactive::coat::Store` for a struct with named fields,
/// generating `{Name}Store`: one `Signal` per field, so a reader of one
/// field does not re-run when another changes.
///
/// See `bevy_radioactive::coat::Store` for details.
#[proc_macro_derive(Store)]
pub fn derive_store(input: proc_macro::TokenStream) -> proc_macro::TokenStream {
    match syn::parse::<syn::DeriveInput>(input) {
        Ok(input) => store(&input)
            .unwrap_or_else(syn::Error::into_compile_error)
            .into(),
        Err(error) => error.into_compile_error().into(),
    }
}

fn store(input: &syn::DeriveInput) -> syn::Result<TokenStream> {
    let syn::Data::Struct(syn::DataStruct {
        fields: syn::Fields::Named(fields),
        ..
    }) = &input.data
    else {
        return Err(syn::Error::new_spanned(
            &input.ident,
            "`#[derive(Store)]` needs a struct with named fields",
        ));
    };
    let coat = quote!(::bevy_radioactive::coat);
    let vis = &input.vis;
    let name = &input.ident;
    let store = format_ident!("{}Store", name);
    let (impl_generics, ty_generics, where_clause) = input.generics.split_for_impl();
    let names: Vec<&Ident> = fields
        .named
        .iter()
        .map(|field| field.ident.as_ref().expect("named field"))
        .collect();
    let types: Vec<&syn::Type> = fields.named.iter().map(|field| &field.ty).collect();
    let visibilities: Vec<&syn::Visibility> = fields.named.iter().map(|field| &field.vis).collect();
    let docs: Vec<String> = names
        .iter()
        .map(|field| format!("The signal of [`{name}::{field}`]."))
        .collect();
    let store_doc = format!(
        "Per-field signals of a [`{name}`], generated by `#[derive(Store)]`.\n\n\
         Reading one field (`store.field.get()`) tracks only that field, and \
         [`set`](Self::set) notifies only the fields whose value changed."
    );

    Ok(quote! {
        #[doc = #store_doc]
        #vis struct #store #impl_generics #where_clause {
            #(
                #[doc = #docs]
                #visibilities #names: #coat::Signal<#types>,
            )*
        }

        impl #impl_generics ::core::clone::Clone for #store #ty_generics #where_clause {
            fn clone(&self) -> Self {
                Self { #(#names: ::core::clone::Clone::clone(&self.#names),)* }
            }
        }

        impl #impl_generics #coat::Store for #name #ty_generics #where_clause {
            type Fields = #store #ty_generics;

            fn into_store(self) -> Self::Fields {
                #store { #(#names: #coat::Signal::new(self.#names),)* }
            }
        }

        impl #impl_generics ::core::convert::From<#name #ty_generics>
            for #store #ty_generics #where_clause
        {
            fn from(value: #name #ty_generics) -> Self {
                #coat::Store::into_store(value)
            }
        }

        #[allow(dead_code)]
        impl #impl_generics #store #ty_generics #where_clause {
            /// Creates the per-field signals of `value`.
            #[must_use]
            pub fn new(value: #name #ty_generics) -> Self {
                #coat::Store::into_store(value)
            }

            /// Clones the whole value, tracking every field.
            #[must_use]
            pub fn get(&self) -> #name #ty_generics {
                #name { #(#names: self.#names.get(),)* }
            }

            /// Clones the whole value without tracking.
            #[must_use]
            pub fn get_untracked(&self) -> #name #ty_generics {
                #name { #(#names: self.#names.get_untracked(),)* }
            }

            /// Writes every field, notifying only the readers of fields
            /// whose value changed. Returns whether any did.
            pub fn set(&self, value: #name #ty_generics) -> bool {
                let mut changed = false;
                #(changed |= self.#names.set_if_neq(value.#names);)*
                changed
            }

            /// Mutates a copy of the whole value and writes it back with
            /// [`set`](Self::set). Returns whether any field changed.
            pub fn update(&self, f: impl ::core::ops::FnOnce(&mut #name #ty_generics)) -> bool {
                let mut value = self.get_untracked();
                f(&mut value);
                self.set(value)
            }
        }
    })
}
