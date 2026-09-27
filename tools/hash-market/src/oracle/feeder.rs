//! HTTP price-bound feeders (Connect-style source → attribute → aggregate).
//!
//! Each configured `kind = "price_bound"` provider polls a ticker URL, upserts
//! an attributed tick, re-aggregates the market, and optionally publishes the
//! single mid into the VE provider map for ABCI++ ExtendVote.

use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::config::ProviderConfig;
#[cfg(feature = "ve")]
use crate::oracle::bound_to_vote_extension;
use crate::oracle::{parse_ticker_json, price_chain_uid, AggregationPolicy, ALGO_PRICE_MEDIAN_V1};
use crate::server::AppState;

fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Poll a ticker URL and feed the attribute store until the task is cancelled.
pub async fn run_price_bound_feeder(
    state: Arc<AppState>,
    provider: ProviderConfig,
    policy: AggregationPolicy,
    runtime_id: String,
    publish_ve: bool,
) {
    let market_id = match provider.market_id.as_deref() {
        Some(m) if !m.is_empty() => m.to_string(),
        _ => {
            tracing::error!(
                provider = %provider.name,
                "price_bound provider missing market_id — feeder not started"
            );
            return;
        }
    };

    let weight = provider.weight.unwrap_or(1.0);
    let interval = std::time::Duration::from_secs(provider.interval_secs.max(1));
    let algo = if provider.algo.is_empty() || provider.algo == "keccak256" {
        ALGO_PRICE_MEDIAN_V1.to_string()
    } else {
        provider.algo.clone()
    };
    let chain_uid = if provider.chain_uid.is_empty() {
        price_chain_uid(&market_id)
    } else {
        provider.chain_uid.clone()
    };

    tracing::info!(
        provider = %provider.name,
        market_id = %market_id,
        url = %provider.address,
        "price_bound feeder starting"
    );

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .unwrap_or_else(|_| reqwest::Client::new());

    loop {
        match fetch_and_ingest(
            &client,
            &state,
            &provider,
            &market_id,
            weight,
            &policy,
            &runtime_id,
            &chain_uid,
            &algo,
            publish_ve,
        )
        .await
        {
            Ok(Some(bound_m)) => {
                tracing::debug!(
                    provider = %provider.name,
                    market = %market_id,
                    mantissa = bound_m,
                    "price_bound aggregate updated"
                );
            }
            Ok(None) => {}
            Err(e) => {
                tracing::warn!(provider = %provider.name, error = %e, "price_bound poll failed");
            }
        }
        tokio::time::sleep(interval).await;
    }
}

async fn fetch_and_ingest(
    client: &reqwest::Client,
    state: &AppState,
    provider: &ProviderConfig,
    default_market: &str,
    weight: f64,
    policy: &AggregationPolicy,
    runtime_id: &str,
    chain_uid: &str,
    algo: &str,
    publish_ve: bool,
) -> anyhow::Result<Option<i128>> {
    let resp = client.get(&provider.address).send().await?;
    if !resp.status().is_success() {
        anyhow::bail!("ticker HTTP {}", resp.status());
    }
    let body = resp.text().await?;
    let now = now_unix();
    let tick = parse_ticker_json(&body, &provider.name, default_market, weight, 8, now)
        .map_err(anyhow::Error::msg)?;

    let store = state
        .oracle_store
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("oracle store not configured"))?;

    let bound = match store.upsert_and_aggregate(tick, policy, now) {
        Ok(b) => b,
        Err(crate::oracle::AggregateError::InsufficientSources { need, have }) => {
            tracing::debug!(
                provider = %provider.name,
                need,
                have,
                "waiting for min_sources"
            );
            return Ok(None);
        }
        Err(e) => return Err(e.into()),
    };

    if publish_ve {
        #[cfg(feature = "ve")]
        {
            use crate::ve::server::{ProviderData, ProviderKey};
            let ve = bound_to_vote_extension(&bound, runtime_id, chain_uid, algo);
            let key = ProviderKey {
                chain_uid: chain_uid.to_string(),
                algo: algo.to_string(),
            };
            state.provider_data.write().await.insert(
                key,
                ProviderData {
                    data: ve,
                    received_at: now,
                },
            );
            // Best-effort status update by name
            let mut statuses = state.provider_status.write().await;
            if let Some(s) = statuses.iter_mut().find(|s| s.name == provider.name) {
                s.running = true;
                s.last_update = Some(now);
                s.foreign_height = Some(bound.as_of);
                s.chain_uid = chain_uid.to_string();
                s.algo = algo.to_string();
            }
        }
        #[cfg(not(feature = "ve"))]
        {
            let _ = (runtime_id, chain_uid, algo);
        }
    }

    Ok(Some(bound.mantissa))
}
