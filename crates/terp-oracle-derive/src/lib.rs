//! `#[oracle_query]`: append the terp-oracle query variants to a `QueryMsg` enum.
//!
//! Place it above `#[cw_serde]` and `#[derive(QueryResponses)]` so the added
//! variants carry their `#[returns(..)]` attributes into both derives (and
//! into `cw_orch::QueryFns` when that derive is present).
//!
//! The macro also emits `impl QueryMsg { fn into_oracle_query(self) }`, which
//! hands the oracle variants to `terp_oracle::handle_oracle_query`.

use proc_macro::TokenStream;
use quote::quote;
use syn::{AttributeArgs, Data, DeriveInput, Fields, Variant, parse_macro_input, parse_quote};

const VARIANTS: [&str; 5] = [
    "OraclePricing",
    "OraclePrice",
    "OracleAggregate",
    "OracleQuote",
    "OracleRaw",
];

#[proc_macro_attribute]
pub fn oracle_query(metadata: TokenStream, input: TokenStream) -> TokenStream {
    let args = parse_macro_input!(metadata as AttributeArgs);
    if let Some(first) = args.first() {
        return syn::Error::new_spanned(first, "#[oracle_query] takes no arguments")
            .to_compile_error()
            .into();
    }
    let mut ast = parse_macro_input!(input as DeriveInput);
    match expand(&mut ast) {
        Ok(ts) => ts.into(),
        Err(e) => e.to_compile_error().into(),
    }
}

fn expand(ast: &mut DeriveInput) -> syn::Result<proc_macro2::TokenStream> {
    let ident = ast.ident.clone();
    let Data::Enum(data) = &mut ast.data else {
        return Err(syn::Error::new_spanned(
            &ast.ident,
            "#[oracle_query] can only be applied to an enum",
        ));
    };
    for v in &data.variants {
        let name = v.ident.to_string();
        if VARIANTS.contains(&name.as_str()) {
            return Err(syn::Error::new_spanned(
                &v.ident,
                format!("variant `{name}` is reserved by #[oracle_query]"),
            ));
        }
    }
    let added: [Variant; 5] = [
        parse_quote! {
            /// Oracle pricing config, if any. `None` means default-asset only.
            #[returns(::core::option::Option<::terp_oracle::OraclePricing>)]
            OraclePricing {}
        },
        parse_quote! {
            /// Combined oracle price of `asset` against `base_denom`, with every feed point.
            #[returns(::terp_oracle::PriceResponse)]
            OraclePrice {
                asset: ::terp_oracle::PriceAsset,
                base_denom: ::std::string::String,
            }
        },
        parse_quote! {
            /// Prices for several pairs in one call. Failed pairs carry an error string.
            #[returns(::terp_oracle::AggregateResponse)]
            OracleAggregate {
                pairs: ::std::vec::Vec<::terp_oracle::AssetPair>,
            }
        },
        parse_quote! {
            /// Amount of `asset` that settles a payment of `base` right now.
            #[returns(::terp_oracle::QuoteResponse)]
            OracleQuote {
                base: ::cosmwasm_std::Coin,
                asset: ::terp_oracle::PriceAsset,
            }
        },
        parse_quote! {
            /// Raw smart query passthrough to one of the configured oracle contracts.
            #[returns(::cosmwasm_std::Binary)]
            OracleRaw {
                oracle: ::std::string::String,
                msg: ::cosmwasm_std::Binary,
            }
        },
    ];
    for v in added {
        data.variants.push(v);
    }

    // Variant field names, used to build the conversion arms.
    let arms = data
        .variants
        .iter()
        .filter(|v| VARIANTS.contains(&v.ident.to_string().as_str()))
        .map(|v| {
            let vi = &v.ident;
            let fields: Vec<_> = match &v.fields {
                Fields::Named(n) => n.named.iter().map(|f| f.ident.clone().unwrap()).collect(),
                _ => vec![],
            };
            quote! {
                #ident::#vi { #(#fields),* } =>
                    ::core::result::Result::Ok(::terp_oracle::OracleQueryMsg::#vi { #(#fields),* }),
            }
        })
        .collect::<Vec<_>>();

    let (impl_g, ty_g, where_c) = ast.generics.split_for_impl();
    Ok(quote! {
        #ast

        impl #impl_g #ident #ty_g #where_c {
            /// Split off the variants added by `#[oracle_query]`.
            /// Returns `Err(self)` for the contract's own variants.
            pub fn into_oracle_query(
                self,
            ) -> ::core::result::Result<::terp_oracle::OracleQueryMsg, Self> {
                match self {
                    #(#arms)*
                    other => ::core::result::Result::Err(other),
                }
            }
        }
    })
}
