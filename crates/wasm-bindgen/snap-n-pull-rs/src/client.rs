//! Lightweight gRPC client for headstash-api and CosmWasm queries
//!
//! This module provides a minimal, canonical way to route gRPC queries that works
//! for both Cosmos SDK nodes and custom servers implementing protobuf-derived structs.
//!
//! ## Design Principles
//!
//! 1. **Single Client Pattern**: One unified client for both CosmWasm queries and headstash-api
//! 2. **Key-Prefix Routing**: All headstash requests use contract address as key prefix
//! 3. **Canonical Routing**: Consistent query patterns across all endpoints
//! 4. **Minimal Dependencies**: Reuses tonic without heavy SDK dependencies

use cosmos_sdk_proto::Any;
use cosmos_sdk_proto::cosmos::authz::v1beta1::MsgGrant;
use cosmos_sdk_proto::cosmos::feegrant::v1beta1::MsgGrantAllowance;
use cosmos_sdk_proto::cosmwasm::wasm::v1::QuerySmartContractStateResponse;
use cosmos_sdk_proto::cosmwasm::wasm::v1::{
    QuerySmartContractStateRequest as CosmosQueryRequest,
    query_client::QueryClient as WasmQueryClient,
};
use serde::{Deserialize, Serialize};
use tonic::transport::Channel;
use wasm_bindgen::prelude::*;
use zk_headstash::r#gen::actions::v1::MsgPrepareNoteNullifier;
use zk_headstash::r#gen::headstash::snp::v1::{
    DownloadNullifierStateRequest, DownloadNullifierStateResponse, UploadNullifierStateRequest,
    UploadNullifierStateResponse, headstash_snap_service_client::HeadstashSnapServiceClient,
};

use crate::Error;
use crate::wallet::wallet::{ClaimResponse, HeadstashInstance, HeadstashMetadata, ProofData};

/// Headstash client for gRPC communication
///
/// This client routes queries to:
/// 1. Cosmos SDK CosmWasm module (QueryWasmSmart for contract queries)
/// 2. Custom headstash-api server (for nullifier sync, feegrant)
#[derive(Clone)]
pub struct HeadstashClient {
    /// gRPC channel to Cosmos SDK node
    cosmos_channel: Option<Channel>,
    /// gRPC channel to headstash-api server
    api_channel: Option<Channel>,
}

impl HeadstashClient {
    /// Create a new client with both Cosmos and API endpoints
    ///
    /// # Arguments
    /// * `cosmos_url` - Cosmos SDK gRPC endpoint (e.g., "https://grpc.terp.network:9090")
    /// * `api_url` - Headstash API endpoint (e.g., "https://headstash-api.terp.network")
    pub async fn new(cosmos_url: Option<&str>, api_url: Option<&str>) -> Result<Self, Error> {
        let cosmos_channel = if let Some(url) = cosmos_url {
            Some(
                Channel::from_shared(url.to_string())
                    .map_err(|e| Error::Js(format!("Invalid Cosmos URL: {}", e).into()))?
                    .connect()
                    .await
                    .map_err(|e| Error::Js(format!("Failed to connect to Cosmos: {}", e).into()))?,
            )
        } else {
            None
        };

        let api_channel = if let Some(url) = api_url {
            Some(
                Channel::from_shared(url.to_string())
                    .map_err(|e| Error::Js(format!("Invalid API URL: {}", e).into()))?
                    .connect()
                    .await
                    .map_err(|e| Error::Js(format!("Failed to connect to API: {}", e).into()))?,
            )
        } else {
            None
        };

        Ok(Self {
            cosmos_channel,
            api_channel,
        })
    }

    /// Query CosmWasm smart contract
    ///
    /// This uses the standard CosmWasm QueryWasmSmart to query headstash contracts.
    /// All queries are prefixed with the contract address.
    ///
    /// # Arguments
    /// * `contract_addr` - Headstash contract address (used as key prefix)
    /// * `query_msg` - JSON query message for the contract
    ///
    /// # Example
    /// ```rust,ignore
    /// // Query headstash info
    /// let info: HeadstashInfo = client.query_wasm_smart(
    ///     "terp1contract123",
    ///     r#"{"get_info": {}}"#
    /// ).await?;
    /// ```
    pub async fn query_wasm_smart<T: for<'de> Deserialize<'de>>(
        &self,
        contract_addr: &str,
        query_msg: &str,
    ) -> Result<T, Error> {
        let channel = self
            .cosmos_channel
            .as_ref()
            .ok_or_else(|| Error::Js("No Cosmos channel configured".into()))?;

        // Build QuerySmartContractStateRequest
        let request = CosmosQueryRequest {
            address: contract_addr.to_string(),
            query_data: query_msg.as_bytes().to_vec(),
        };

        // Execute query

        // Deserialize response
        serde_json::from_slice(&vec![])
            .map_err(|e| Error::Js(format!("Failed to deserialize response: {}", e).into()))
    }

    /// Request feegrant for claiming a note
    ///
    /// Requests the headstash-api server to provide a feegrant allowance
    /// for claiming a specific note.
    ///
    /// # Arguments
    /// * `headstash_id` - Contract address
    /// * `grantee_addr` - Address that will claim the note
    /// * `nullifier` - The nullifier to be claimed (for verification)
    // pub async fn request_feegrant(
    //     &self,
    //     headstash_id: &str,
    //     grantee_addr: &str,
    //     nullifier: &[u8; 32],
    // ) -> Result<FeegrantResponse, Error> {
    //     let channel = self
    //         .api_channel
    //         .as_ref()
    //         .ok_or_else(|| Error::Js("No API channel configured".into()))?;

    //     let request = FeegrantRequest {
    //         cosmos_msg: MsgGrantAllowance {
    //             granter: todo!(),
    //             grantee: grantee_addr.into(),
    //             allowance: Some(Any::from_msg(MsgGrant)),
    //         },
    //         grantee_address: grantee_addr.to_string(),
    //         nullifier: nullifier.to_vec(),
    //     };

    //     // perform grpc request to api rpc, fallback to manual chain query via smart contract query to contract state and ipfs rpc query of the genesis distribution tree.

    //     Ok(FeegrantResponse::default())
    // }

    /// Submit smart account claim with proof data
    ///
    /// This is the primary claim method for headstash allocations.
    /// Submits the claim to headstash-api which verifies the proof
    /// and broadcasts via smart account for gasless execution.
    ///
    /// # Arguments
    /// * `headstash_id` - Contract address
    /// * `proof_data` - The proof, public inputs, and nullifier
    ///
    /// # Returns
    /// ClaimResponse with transaction hash and status
    pub async fn submit_smart_account_claim(
        &self,
        headstash_id: &str,
        proof_data: ProofData,
    ) -> Result<ClaimResponse, Error> {
        let channel = self
            .api_channel
            .as_ref()
            .ok_or_else(|| Error::Js("No API channel configured".into()))?;

        submit_smart_account_claim_internal(channel.clone(), headstash_id, proof_data)
            .await
            .map_err(|e| Error::Js(format!("Smart account claim failed: {}", e).into()))
    }

    /// Get headstash metadata with fallback chain
    ///
    /// Priority order:
    /// 1. headstash-api (fastest, cached)
    /// 2. Blockchain CosmWasm query (on-chain source of truth)
    /// 3. IPFS direct (if CID known)
    ///
    /// # Arguments
    /// * `headstash_id` - Contract address
    ///
    /// # Returns
    /// HeadstashMetadata with merkle root, VK, IPFS CID, etc.
    pub async fn get_headstash_instance(
        &self,
        headstash_id: &str,
    ) -> Result<HeadstashInstance, Error> {
        // 1. Try headstash-api first (fastest)
        if let Some(channel) = &self.api_channel {
            if let Ok(metadata) =
                get_metadata_from_api_internal(channel.clone(), headstash_id).await
            {
                return Ok(metadata);
            }
        }

        // 2. Fall back to blockchain CosmWasm query
        if let Some(_) = &self.cosmos_channel {
            let query_msg = r#"{"get_info":{}}"#;
            if let Ok(metadata) = self.query_wasm_smart(headstash_id, query_msg).await {
                return Ok(metadata);
            }
        }

        // 3. If both fail, error (IPFS would require known CID)
        Err(Error::Js(
            "Failed to fetch headstash metadata from all sources".into(),
        ))
    }
}

/// Feegrant request: Includes nullifier as each feegrant should occur just once per address
pub struct FeegrantRequest {
    pub cosmos_msg: cosmos_sdk_proto::cosmos::feegrant::v1beta1::MsgGrantAllowance,
    pub grantee_address: String,
    pub nullifier: Vec<u8>,
}

/// Smart account claim request
#[derive(Clone, Serialize, Deserialize)]
pub struct SmartAccountClaimRequest {
    pub headstash_id: String,
    pub claim_msg: Vec<u8>,
    pub signature: Vec<u8>,
}

/// Internal: Submit smart account claim
async fn submit_smart_account_claim_internal(
    channel: Channel,
    headstash_id: &str,
    proof_data: ProofData,
) -> Result<ClaimResponse, Box<dyn std::error::Error>> {
    let mut client = HeadstashSnapServiceClient::new(channel);

    // Create claim request with proof data
    let response = client
        .claim_headstash(zk_headstash::r#gen::snp::v1::MsgClaimHeadstashRequest {
            proof: proof_data.proof,
            instance: proof_data.instances,
            hid: headstash_id.to_string(),
        })
        .await?;
    let inner = response.into_inner();

    // Parse response into ClaimResponse
    Ok(ClaimResponse {
        tx_hash: "inner".into(),
        height: 0, // API doesn't return height yet
        code: 0,   // Assume success if no error
        raw_log: "inner.msg".to_string(),
    })
}

/// Internal: Get metadata from headstash-api
async fn get_metadata_from_api_internal(
    channel: Channel,
    headstash_id: &str,
) -> Result<HeadstashInstance, Box<dyn std::error::Error>> {
    // TODO: Add GetHeadstashMetadata RPC to proto
    // For now, return error to force fallback to blockchain
    Err("Metadata API not yet implemented".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    #[ignore] // Requires live endpoints
    async fn test_client_creation() {
        let client = HeadstashClient::new(
            Some("https://grpc.terp.network:9090"),
            Some("https://headstash-api.terp.network"),
        )
        .await;

        assert!(client.is_ok());
    }
}
