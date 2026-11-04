//! Offline: check the fractal template against every contract validator, then render
//! sample tokens through the real cw721-svg code (library, mock deps) to <out>/.
//! cargo run -p cw-infuser-scripts --bin fractal_preview -- <out_dir> [n]
use cosmwasm_std::testing::{message_info, mock_dependencies, mock_env};
use cw_infuser_scripts::fractal::*;
fn main() -> anyhow::Result<()> {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let out = std::path::PathBuf::from(a.first().cloned().unwrap_or("out/fractal".into()));
    let n: u64 = a.get(1).map(|x| x.parse()).transpose()?.unwrap_or(6);
    std::fs::create_dir_all(&out)?;
    let mut deps = mock_dependencies();
    let curator = deps.api.addr_make("curator");
    let roles = FractalRoles {
        curator: curator.to_string(),
        withdraw: curator.to_string(),
        paymint_whitelist: None,
    };
    let mut init = permissionless_fractal_init_v2(&42, &roles)?;
    println!(
        "check_template ok; packed {} bytes (template {} B)",
        init.collection_info_extension.packed.len(),
        FRACTAL_SVG_TEMPLATE.len()
    );
    init.collection_info_extension.price_tiers = vec![];
    cw721_svg::entry::instantiate(deps.as_mut(), mock_env(), message_info(&curator, &[]), init)
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    for i in 0..n {
        let mut env = mock_env();
        env.block.height += i * 7919;
        cw721_svg::entry::execute(
            deps.as_mut(),
            env.clone(),
            message_info(&curator, &[]),
            cw721_svg::ExecuteMsg::UpdateExtension {
                msg: cw721_svg::SvgExecuteMsgExt::Mint(cw_svg::MintMsg {
                    alloc: 0,
                    amnt: 1,
                    proof_hashes: vec![],
                    ext: None,
                }),
            },
        )
        .map_err(|e| anyhow::anyhow!("{e}"))?;
        let q = cw721_svg::entry::query(
            deps.as_ref(),
            env,
            cw721_svg::QueryMsg::Extension {
                msg: cw721_svg::SvgQueryMsgExt::SvgTokenUri {
                    token_id: i.to_string(),
                },
            },
        )
        .map_err(|e| anyhow::anyhow!("{e}"))?;
        let r: cw721_svg::contract::SvgTokenUriResponse =
            cosmwasm_std::from_json(q).map_err(|e| anyhow::anyhow!("{e}"))?;
        let p = out.join(format!("fractal-{i}.svg"));
        std::fs::write(&p, &r.svg)?;
        println!("{} ({} B)", p.display(), r.svg.len());
    }
    Ok(())
}
