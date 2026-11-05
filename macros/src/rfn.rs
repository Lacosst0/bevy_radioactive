//! `#[rfn]`: a scene function that takes Bevy system parameters as
//! arguments, like a system, and reads them in `rsn!` as `$name`.
//!
//! The function is split at compile time, the way Svelte's compiler splits a
//! component script:
//!
//! - An argument whose type is a system parameter (`Res<T>`, `Query<..>`,
//!   `Single<..>`, `Untracked<P>`, ..) is reactive and is
//!   removed from the signature. `#[from(target)] x: Ref<T>` reads component
//!   `T` on an entity (`"Name"` or an `Entity` argument). Other arguments
//!   are plain props.
//! - A `let` whose value mentions a reactive name is *derived*: it is
//!   reactive too.
//! - Every other statement runs once, when the function is called.
//!
//! Reactive arguments and `let`s are removed. Each `$name` in a scene macro
//! (`rsn!`, `rsn_list!`, `bsn!`, `bsn_list!`) is replaced by what it stands
//! for: `$Param` for a parameter, or a block that reads the parameters a
//! derived value needs and recomputes it. `rsn!` then turns each use into a
//! closure that re-runs when those parameters change. Plain locals and
//! arguments are cloned per use, so they can be used any number of times.

use proc_macro2::{Delimiter, Group, Ident, Spacing, Span, TokenStream, TokenTree};
use quote::{ToTokens, quote, quote_spanned};
use syn::{Pat, Stmt, Type};

use super::{Tokens, is_ident, is_punct, tokens_of};

/// One reactive argument or `let`.
struct Reactive {
    /// The names it binds.
    names: Vec<Ident>,
    kind: Kind,
    /// Earlier reactive items it reads (indices), for derived values.
    deps: Vec<usize>,
}

enum Kind {
    /// An argument `name: Ty` or `#[from(target)] name: Ref<T>`: `$Ty` or `$Ref<T>(target)`.
    Param {
        ty: TokenStream,
        target: Option<TokenStream>,
        /// `Res`, `Query`, ..: the type's last path segment.
        wrapper: String,
    },
    /// `let pat = expr;` mentioning reactive names.
    Derived(TokenStream),
}

/// Parameters that dereference to the value they read.
const DEREF: &[&str] = &[
    "Res",
    "ResMut",
    "Single",
    "Ref",
    "Mut",
    "Local",
    "NonSend",
    "NonSendMut",
];

const SCENE_MACROS: &[&str] = &["rsn", "rsn_list", "bsn", "bsn_list"];

pub(crate) fn expand(attr: TokenStream, item: TokenStream) -> TokenStream {
    if !attr.is_empty() {
        return syn::Error::new(Span::call_site(), "`#[rfn]` takes no arguments")
            .into_compile_error();
    }
    let mut function: syn::ItemFn = match syn::parse2(item) {
        Ok(function) => function,
        Err(error) => return error.into_compile_error(),
    };

    // System parameter arguments are reactive; the others are plain props.
    let mut plain = Vec::new();
    let mut reactive: Vec<Reactive> = Vec::new();
    let mut errors = TokenStream::new();
    for input in std::mem::take(&mut function.sig.inputs) {
        let syn::FnArg::Typed(mut arg) = input else {
            function.sig.inputs.push(input);
            continue;
        };
        let from = take_attr(&mut arg.attrs, "from");
        if from.is_none() && !is_param(&arg.ty) {
            pattern_names(&arg.pat, &mut plain);
            function.sig.inputs.push(syn::FnArg::Typed(arg));
            continue;
        }
        let mut names = Vec::new();
        pattern_names(&arg.pat, &mut names);
        let wrapper = last_ident(&arg.ty).unwrap_or_default();
        let component = wrapper == "Ref" || wrapper == "Mut";
        let target = match (&from, component) {
            (Some(attr), true) => match attr.parse_args::<syn::Expr>() {
                Ok(expr) => Some(expr.to_token_stream()),
                Err(error) => {
                    error.into_compile_error().to_tokens(&mut errors);
                    continue;
                }
            },
            (None, true) => {
                syn::Error::new_spanned(
                    &arg,
                    "say which entity: `#[from(\"Name\")] x: Ref<T>` for the nearest entity \
                     named `Name`, or `#[from(entity)]` for an `Entity` argument",
                )
                .into_compile_error()
                .to_tokens(&mut errors);
                continue;
            }
            (Some(attr), false) => {
                syn::Error::new_spanned(
                    attr,
                    "`#[from(..)]` is for `Ref<T>` and `Mut<T>` arguments",
                )
                .into_compile_error()
                .to_tokens(&mut errors);
                continue;
            }
            (None, false) => None,
        };
        let mut ty = (*arg.ty).clone();
        if component && let Type::Path(path) = &mut ty {
            // `Mut<T>` and `Ref<T>` read the same; `on(..)` writes either.
            let last = path.path.segments.last_mut().unwrap();
            last.ident = Ident::new("Ref", last.ident.span());
        }
        reactive.push(Reactive {
            names,
            kind: Kind::Param {
                ty: ty.to_token_stream(),
                target,
                wrapper: if component { "Ref".into() } else { wrapper },
            },
            deps: Vec::new(),
        });
    }

    let mut kept = Vec::new();
    for stmt in std::mem::take(&mut function.block.stmts) {
        let Stmt::Local(local) = &stmt else {
            kept.push(stmt);
            continue;
        };
        let mut names = Vec::new();
        pattern_names(&local.pat, &mut names);
        if let Pat::Type(typed) = &local.pat
            && is_param(&typed.ty)
        {
            syn::Error::new_spanned(
                &local.pat,
                "declare system parameters as function arguments: `fn scene(score: Res<Score>)`",
            )
            .into_compile_error()
            .to_tokens(&mut errors);
            continue;
        }
        match &local.init {
            Some(init) => {
                let mut deps = Vec::new();
                let value: Tokens = init.expr.to_token_stream().into_iter().collect();
                let diverge: Tokens = init
                    .diverge
                    .as_ref()
                    .map(|(_, e)| e.to_token_stream().into_iter().collect())
                    .unwrap_or_default();
                for (index, item) in reactive.iter().enumerate() {
                    if mentions(&value, &item.names) || mentions(&diverge, &item.names) {
                        deps.push(index);
                    }
                }
                if deps.is_empty() {
                    plain.extend(names);
                    kept.push(stmt);
                    continue;
                }
                // Transitive: a derived value reads what its inputs read.
                let mut all = Vec::new();
                for dep in deps {
                    for inner in &reactive[dep].deps {
                        if !all.contains(inner) {
                            all.push(*inner);
                        }
                    }
                    if !all.contains(&dep) {
                        all.push(dep);
                    }
                }
                all.sort_unstable();
                // A plain name this `let` shadows is now reactive.
                plain.retain(|name| !names.contains(name));
                reactive.push(Reactive {
                    names,
                    kind: Kind::Derived(stmt.to_token_stream()),
                    deps: all,
                });
            }
            None => kept.push(stmt),
        }
    }

    let context = Context {
        reactive: &reactive,
        plain: &plain,
    };
    let mut body = TokenStream::new();
    for stmt in kept {
        let tokens: Tokens = stmt.to_token_stream().into_iter().collect();
        body.extend(context.rewrite(&tokens));
    }
    let attrs = &function.attrs;
    let vis = &function.vis;
    let sig = &function.sig;
    quote! {
        #errors
        #(#attrs)*
        #vis #sig {
            #body
        }
    }
}

/// The system parameter types that make an argument reactive.
const PARAMS: &[&str] = &[
    "Res",
    "ResMut",
    "Query",
    "Single",
    "Populated",
    "Local",
    "Commands",
    "NonSend",
    "NonSendMut",
    "Ref",
    "Mut",
    "MessageReader",
    "MessageWriter",
    "Untracked",
];

/// The last path segment of a type: `Res` for `bevy::prelude::Res<Score>`.
fn last_ident(ty: &Type) -> Option<String> {
    let Type::Path(path) = ty else { return None };
    Some(path.path.segments.last()?.ident.to_string())
}

/// Whether `ty` is a known system parameter, or an `Option` of one.
fn is_param(ty: &Type) -> bool {
    let Type::Path(path) = ty else { return false };
    let Some(last) = path.path.segments.last() else {
        return false;
    };
    let name = last.ident.to_string();
    if name == "Option"
        && let syn::PathArguments::AngleBracketed(args) = &last.arguments
        && let Some(syn::GenericArgument::Type(inner)) = args.args.first()
    {
        return is_param(inner);
    }
    PARAMS.contains(&name.as_str())
}

/// Removes `#[name(..)]` from `attrs` and returns it.
fn take_attr(attrs: &mut Vec<syn::Attribute>, name: &str) -> Option<syn::Attribute> {
    let index = attrs.iter().position(|a| a.path().is_ident(name))?;
    Some(attrs.remove(index))
}

fn pattern_names(pat: &Pat, out: &mut Vec<Ident>) {
    match pat {
        Pat::Ident(p) => {
            out.push(p.ident.clone());
            if let Some((_, sub)) = &p.subpat {
                pattern_names(sub, out);
            }
        }
        Pat::Type(p) => pattern_names(&p.pat, out),
        Pat::Tuple(p) => p.elems.iter().for_each(|p| pattern_names(p, out)),
        Pat::TupleStruct(p) => p.elems.iter().for_each(|p| pattern_names(p, out)),
        Pat::Struct(p) => p.fields.iter().for_each(|f| pattern_names(&f.pat, out)),
        Pat::Slice(p) => p.elems.iter().for_each(|p| pattern_names(p, out)),
        Pat::Reference(p) => pattern_names(&p.pat, out),
        Pat::Paren(p) => pattern_names(&p.pat, out),
        Pat::Or(p) => p.cases.iter().for_each(|p| pattern_names(p, out)),
        _ => {}
    }
}

fn mentions(tokens: &[TokenTree], names: &[Ident]) -> bool {
    super::rsn::mentions(tokens, names)
}

struct Context<'a> {
    reactive: &'a [Reactive],
    plain: &'a [Ident],
}

impl Context<'_> {
    fn find(&self, name: &Ident) -> Option<usize> {
        // The last declaration wins, as with shadowing.
        self.reactive.iter().rposition(|r| r.names.contains(name))
    }

    /// Whether `tokens[i]`, a reactive name, is used as a value (not a
    /// field, path segment, `$name` or `#Name`).
    fn bare_use(tokens: &[TokenTree], i: usize) -> bool {
        let before = i.checked_sub(1).and_then(|j| tokens.get(j));
        let after = tokens.get(i + 1);
        let field = matches!(after, Some(TokenTree::Punct(p)) if p.as_char() == ':' && p.spacing() == Spacing::Alone)
            && !is_punct(before, '.');
        !(is_punct(before, '$')
            || is_punct(before, '.')
            || is_punct(before, '#')
            || is_punct(before, ':')
            || is_punct(before, '@')
            || field
            || is_punct(after, '!'))
    }

    /// The body outside scene macros: rewrites each scene macro and rejects
    /// other uses of reactive names.
    fn rewrite(&self, tokens: &[TokenTree]) -> TokenStream {
        let mut out = TokenStream::new();
        let mut i = 0;
        while i < tokens.len() {
            if let Some((name, body, list)) = scene_macro(tokens, i) {
                self.scene(name, body, list).to_tokens(&mut out);
                i += 3;
                continue;
            }
            match &tokens[i] {
                TokenTree::Ident(ident)
                    if self.find(ident).is_some() && Self::bare_use(tokens, i) =>
                {
                    let message = format!(
                        "`{ident}` is reactive, so it has no value in the function body: \
                         read it as `${ident}` in `rsn!`, or use it in a `let` (which becomes \
                         reactive too)"
                    );
                    quote_spanned!(ident.span()=> ::core::compile_error!(#message))
                        .to_tokens(&mut out);
                }
                TokenTree::Group(g) => {
                    let mut group = Group::new(g.delimiter(), self.rewrite(&tokens_of(g)));
                    group.set_span(g.span());
                    group.to_tokens(&mut out);
                }
                other => other.to_tokens(&mut out),
            }
            i += 1;
        }
        out
    }

    /// `rsn! { .. }` with `$name`s substituted, expanded with the plain
    /// names cloned per use.
    fn scene(&self, name: &Ident, body: &Group, list: bool) -> TokenStream {
        let body = self.substitute(&tokens_of(body), false);
        let kind = Ident::new(if list { "list" } else { "item" }, name.span());
        let plain = self.plain;
        let span = name.span();
        quote_spanned!(span=> ::bevy_radioactive::macros::__rfn_rsn! { #kind [#(#plain),*] #body })
    }

    /// Replaces each `$name` in a scene macro body, deeply. In `on(..)`,
    /// resources become writable.
    fn substitute(&self, tokens: &[TokenTree], handler: bool) -> TokenStream {
        let mut out = TokenStream::new();
        let mut i = 0;
        while i < tokens.len() {
            match &tokens[i] {
                TokenTree::Punct(p) if p.as_char() == '$' => {
                    if let Some(TokenTree::Ident(name)) = tokens.get(i + 1)
                        && let Some(index) = self.find(name)
                    {
                        self.value(index, name, handler).to_tokens(&mut out);
                        i += 2;
                        continue;
                    }
                    if let Some(TokenTree::Ident(name)) = tokens.get(i + 1)
                        && name
                            .to_string()
                            .starts_with(|c: char| c.is_lowercase() || c == '_')
                    {
                        let message = if self.plain.contains(name) {
                            format!(
                                "`{name}` isn't reactive (its value doesn't use a reactive name): use it without `$`"
                            )
                        } else {
                            format!(
                                "no reactive `{name}` here: add it as a function argument, like `{name}: Res<T>`"
                            )
                        };
                        quote_spanned!(name.span()=> ::core::compile_error!(#message))
                            .to_tokens(&mut out);
                        i += 2;
                        continue;
                    }
                    p.to_tokens(&mut out);
                }
                TokenTree::Ident(ident)
                    if self.find(ident).is_some() && Self::bare_use(tokens, i) =>
                {
                    let message = format!("`{ident}` is reactive: read it as `${ident}`");
                    quote_spanned!(ident.span()=> ::core::compile_error!(#message))
                        .to_tokens(&mut out);
                }
                TokenTree::Ident(ident) => {
                    // A nested `bsn!` reading `$name` is reactive: make it `rsn!`.
                    match scene_macro(tokens, i) {
                        Some((_, g, list)) if has_dollar(&tokens_of(g)) => {
                            let name =
                                Ident::new(if list { "rsn_list" } else { "rsn" }, ident.span());
                            quote!(::bevy_radioactive::macros::#name!).to_tokens(&mut out);
                            let mut group =
                                Group::new(g.delimiter(), self.substitute(&tokens_of(g), false));
                            group.set_span(g.span());
                            group.to_tokens(&mut out);
                            i += 3;
                            continue;
                        }
                        _ => ident.to_tokens(&mut out),
                    }
                    // `on(..)`: a handler, where reads may write.
                    if ident == "on"
                        && !is_punct(i.checked_sub(1).and_then(|j| tokens.get(j)), '.')
                        && let Some(TokenTree::Group(g)) = tokens.get(i + 1)
                        && g.delimiter() == Delimiter::Parenthesis
                    {
                        let mut group =
                            Group::new(g.delimiter(), self.substitute(&tokens_of(g), true));
                        group.set_span(g.span());
                        group.to_tokens(&mut out);
                        i += 2;
                        continue;
                    }
                }
                TokenTree::Group(g) => {
                    let mut group =
                        Group::new(g.delimiter(), self.substitute(&tokens_of(g), handler));
                    group.set_span(g.span());
                    group.to_tokens(&mut out);
                }
                other => other.to_tokens(&mut out),
            }
            i += 1;
        }
        out
    }

    /// What `$name` stands for.
    fn value(&self, index: usize, name: &Ident, handler: bool) -> TokenStream {
        let item = &self.reactive[index];
        match &item.kind {
            Kind::Param { .. } => self.param(item, handler),
            Kind::Derived(_) => {
                let mut lets = TokenStream::new();
                for &dep in &item.deps {
                    let dep = &self.reactive[dep];
                    match &dep.kind {
                        Kind::Param { wrapper, .. } => {
                            let read = self.param(dep, handler);
                            // Smart pointers read as what they point to
                            // (`&Score` for `Res<Score>`), others by reference.
                            let read = if DEREF.contains(&wrapper.as_str()) {
                                quote!(&*#read)
                            } else {
                                quote!(&#read)
                            };
                            for name in &dep.names {
                                quote!(#[allow(unused_variables)] let #name = #read;)
                                    .to_tokens(&mut lets);
                            }
                        }
                        Kind::Derived(stmt) => stmt.to_tokens(&mut lets),
                    }
                }
                let Kind::Derived(stmt) = &item.kind else {
                    unreachable!()
                };
                let span = name.span();
                quote_spanned!(span=> ({ #lets #stmt #name }))
            }
        }
    }

    /// `$Ty` or `$Ref<T>(target)`; `Res` reads become `ResMut` in a handler.
    fn param(&self, item: &Reactive, handler: bool) -> TokenStream {
        let Kind::Param { ty, target, .. } = &item.kind else {
            unreachable!()
        };
        let mut ty: Tokens = ty.clone().into_iter().collect();
        if handler
            && let Some(TokenTree::Ident(first)) = ty.first()
            && first == "Res"
        {
            ty[0] = TokenTree::Ident(Ident::new("ResMut", first.span()));
        }
        let ty: TokenStream = ty.into_iter().collect();
        match target {
            Some(target) => quote!($#ty(#target)),
            None => quote!($#ty),
        }
    }
}

fn has_dollar(tokens: &[TokenTree]) -> bool {
    tokens.iter().any(|token| match token {
        TokenTree::Punct(p) => p.as_char() == '$',
        TokenTree::Group(g) => has_dollar(&tokens_of(g)),
        _ => false,
    })
}

/// `name! { .. }` at `tokens[i]` for a scene macro: its name, body and
/// whether it's the list form.
fn scene_macro(tokens: &[TokenTree], i: usize) -> Option<(&Ident, &Group, bool)> {
    let TokenTree::Ident(name) = tokens.get(i)? else {
        return None;
    };
    let name_str = name.to_string();
    if !SCENE_MACROS.contains(&name_str.as_str()) || !is_punct(tokens.get(i + 1), '!') {
        return None;
    }
    // Not `foo.rsn!` or `other::rsn!`.
    if i > 0 && (is_punct(tokens.get(i - 1), '.') || is_punct(tokens.get(i - 1), ':')) {
        // `bevy::scene::bsn!` paths are fine, but rare; leave them alone.
        return None;
    }
    let Some(TokenTree::Group(body)) = tokens.get(i + 2) else {
        return None;
    };
    Some((name, body, name_str.ends_with("_list")))
}

/// `__rfn_rsn! { item|list [plain, names] body }`: `rsn!` with plain names
/// cloned per use.
pub(crate) fn rsn_with(input: TokenStream) -> TokenStream {
    let tokens: Tokens = input.into_iter().collect();
    let list = is_ident(tokens.first(), "list");
    let Some(TokenTree::Group(names)) = tokens.get(1) else {
        return quote!(::core::compile_error!("internal: malformed `__rfn_rsn!`"));
    };
    let bound = tokens_of(names)
        .into_iter()
        .filter_map(|t| match t {
            TokenTree::Ident(ident) => Some(ident),
            _ => None,
        })
        .collect();
    let body: TokenStream = tokens[2..].iter().cloned().collect();
    super::rsn::expand_with(body, list, bound)
}
