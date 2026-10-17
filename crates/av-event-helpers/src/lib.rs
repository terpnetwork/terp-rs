use cosmwasm_std::{coin, Addr, Coin, StdResult};

pub const LICENSE_CANONICAL_ADDR: &str = "58855806243FE9F4FB4023C8D149DF9AF1C3891E";

pub fn get_license_fee(chain_id: &str) -> StdResult<Coin> {
    match chain_id {
        // Local terpd/wasmd lab (itlusher). Do not copy juno/morocco code ids.
        "cw-ave-lab" | "local-1" | "testing" | "testnet-1" | "240u-2" | "itl-cashapp-1" => {
            Ok(coin(1_000_000u128, "uthiol"))
        }
        "bitsong-2b" => Ok(coin(420_000_000u128, "ubtsg")),
        "juno-1" => Ok(coin(1_000_000, "ujuno")),
        "cosmoshub-4" => Ok(coin(420_000_000u128, "uatom")),
        "neutron-1" => Ok(coin(420_000_000u128, "untrn")),
        "archway-1" => Ok(coin(420_000_000u128, "aarch")),
        "bitcanna-1" => Ok(coin(420_000_000u128, "ubcna")),
        "chihuahua-1" => Ok(coin(420_000_000u128, "uhuahua")),
        "omniflixhub-1" => Ok(coin(420_000_000u128, "uflix")),
        "secret-4" => Ok(coin(420_000_000u128, "uscrt")),
        "migaloo-1" => Ok(coin(420_000_000u128, "uwhale")),
        "columbus-5" => Ok(coin(420_000_000u128, "uluna")),
        "phoenix-1" => Ok(coin(420_000_000u128, "uluna")),
        "kaiyo-1" => Ok(coin(420_000_000u128, "ukuji")),
        "luwak-1" => Ok(coin(420_000_000u128, "ukopi")),
        "aaronetwork" => Ok(coin(420_000_000u128, "uaaron")),
        "acre_9052-1" => Ok(coin(420_000_000u128, "aacre")),
        _ => Err(cosmwasm_std::StdError::msg(
            "no license fee for this chain ",
        )),
    }
}

pub fn get_license_addr(chain_id: &str) -> StdResult<Addr> {
    match chain_id {
        "juno-1" => Ok(Addr::unchecked(
            "juno1tzz4sp3y8l5lf76qy0ydzjwlntcu8zg7t63p68",
        )),
        "bitsong-2b" => Ok(Addr::unchecked(
            "bitsong1schul8k23ryty6ar324mzee0axjx0rxec5t6hk",
        )),
        // terp bech32 of LICENSE_CANONICAL_ADDR (same 20-byte payload as juno1tzz4…)
        "cw-ave-lab" | "local-1" | "testing" | "testnet-1" | "240u-2" | "itl-cashapp-1" => {
            Ok(Addr::unchecked(
                "terp1tzz4sp3y8l5lf76qy0ydzjwlntcu8zg76zmkhg",
            ))
        }
        _ => Err(cosmwasm_std::StdError::msg(
            "no license address for this chain ",
        )),
    }
}
