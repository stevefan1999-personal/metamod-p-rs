//! `#[metamod_p::plugin]`, re-exported by the `metamod-p` crate.

use proc_macro::{Delimiter, Group, TokenStream, TokenTree};

/// Exports an `impl Plugin for YourType { ... }` and installs only the hook
/// methods that the impl overrides.
///
/// The attribute adds `const OVERRIDES` with the names of the methods in
/// the impl and calls `metamod_p::export_plugin!` for the type, so that
/// metamod never calls the plugin for hooks it does not use.
#[proc_macro_attribute]
pub fn plugin(attribute: TokenStream, item: TokenStream) -> TokenStream {
    expand(attribute, item).unwrap_or_else(|message| {
        format!("::core::compile_error!({message:?});")
            .parse()
            .expect("compile_error! tokens")
    })
}

const USAGE: &str = "#[metamod_p::plugin] goes on `impl Plugin for YourType { ... }`";

fn expand(attribute: TokenStream, item: TokenStream) -> Result<TokenStream, String> {
    if !attribute.is_empty() {
        return Err("#[metamod_p::plugin] takes no arguments".into());
    }
    let mut tokens: Vec<TokenTree> = item.into_iter().collect();
    let body = match tokens.pop() {
        Some(TokenTree::Group(body)) if body.delimiter() == Delimiter::Brace => body,
        _ => return Err(USAGE.into()),
    };
    let plugin_type: TokenStream = tokens
        .iter()
        .position(|token| matches!(token, TokenTree::Ident(ident) if ident.to_string() == "for"))
        .map(|position| tokens[position + 1..].iter().cloned().collect())
        .ok_or(USAGE)?;
    // Methods sit at the top level of the impl body; method bodies are
    // nested groups, so their own `fn` items never show up here.
    let items: Vec<TokenTree> = body.stream().into_iter().collect();
    let methods: Vec<String> = items
        .windows(2)
        .filter_map(|pair| {
            match pair {
                [TokenTree::Ident(keyword), TokenTree::Ident(name)]
                    if keyword.to_string() == "fn" =>
                {
                    Some(format!("{:?}", name.to_string()))
                }
                _ => None,
            }
        })
        .collect();
    let overrides: TokenStream = format!(
        "const OVERRIDES: ::metamod_p::Overrides = ::metamod_p::Overrides::Only(&[{}]);",
        methods.join(", ")
    )
    .parse()
    .map_err(|error| format!("{error}"))?;
    let mut impl_body = body.stream();
    impl_body.extend(overrides);
    let mut impl_group = Group::new(Delimiter::Brace, impl_body);
    impl_group.set_span(body.span());
    tokens.push(TokenTree::Group(impl_group));
    let export: TokenStream = format!("::metamod_p::export_plugin!({plugin_type});")
        .parse()
        .map_err(|error| format!("{error}"))?;
    Ok(tokens.into_iter().chain(export).collect())
}
