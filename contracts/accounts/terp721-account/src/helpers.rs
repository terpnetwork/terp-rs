use cosmwasm_schema::cw_serde;
use cw721::msg::{
    AllNftInfoResponse, ApprovalResponse, NftInfoResponse, OperatorResponse, OperatorsResponse,
    OwnerOfResponse,
};
use cw721::Approval;
use serde::de::DeserializeOwned;
use terp_account::Metadata;

use cosmwasm_std::{
    to_json_binary, Addr, CosmosMsg, QuerierWrapper, StdResult, WasmMsg, WasmQuery,
};

use crate::msg::{ExecuteMsg, Terp721AccountsQueryMsg};

#[cw_serde]
pub struct Terp721Account(pub Addr);

impl Terp721Account {
    pub fn addr(&self) -> Addr {
        self.0.clone()
    }

    pub fn call<T: Into<ExecuteMsg<Metadata>>>(&self, msg: T) -> StdResult<CosmosMsg> {
        let msg = to_json_binary(&msg.into())?;
        Ok(WasmMsg::Execute {
            contract_addr: self.addr().into(),
            msg,
            funds: vec![],
        }
        .into())
    }

    pub fn query<T: DeserializeOwned>(
        &self,
        querier: &QuerierWrapper,
        req: Terp721AccountsQueryMsg,
    ) -> StdResult<T> {
        let query = WasmQuery::Smart {
            contract_addr: self.addr().into(),
            msg: to_json_binary(&req)?,
        }
        .into();
        querier.query(&query)
    }

    pub fn operator<T: Into<String>>(
        &self,
        querier: &cosmwasm_std::QuerierWrapper,
        owner: T,
        operator: T,
        include_expired: bool,
    ) -> StdResult<Approval> {
        let req = Terp721AccountsQueryMsg::Operator {
            owner: owner.into(),
            operator: operator.into(),
            include_expired: Some(include_expired),
        };
        let res: OperatorResponse = self.query(querier, req)?;
        Ok(res.approval)
    }

    /// With metadata extension
    pub fn nft_info<T: Into<String>, U: DeserializeOwned>(
        &self,
        querier: &QuerierWrapper,
        token_id: T,
    ) -> StdResult<NftInfoResponse<U>> {
        let req = Terp721AccountsQueryMsg::NftInfo {
            token_id: token_id.into(),
        };
        self.query(querier, req)
    }

    /// With metadata extension
    pub fn all_nft_info<T: Into<String>, U: DeserializeOwned>(
        &self,
        querier: &QuerierWrapper,
        token_id: T,
        include_expired: bool,
    ) -> StdResult<AllNftInfoResponse<U>> {
        let req = Terp721AccountsQueryMsg::AllNftInfo {
            token_id: token_id.into(),
            include_expired: Some(include_expired),
        };
        self.query(querier, req)
    }

    pub fn owner_of<T: Into<String>>(
        &self,
        querier: &QuerierWrapper,
        token_id: T,
        include_expired: bool,
    ) -> StdResult<OwnerOfResponse> {
        let req = Terp721AccountsQueryMsg::OwnerOf {
            token_id: token_id.into(),
            include_expired: Some(include_expired),
        };
        self.query(querier, req)
    }

    pub fn approval<T: Into<String>>(
        &self,
        querier: &QuerierWrapper,
        token_id: T,
        spender: T,
        include_expired: Option<bool>,
    ) -> StdResult<ApprovalResponse> {
        let req = Terp721AccountsQueryMsg::Approval {
            token_id: token_id.into(),
            spender: spender.into(),
            include_expired,
        };
        let res: ApprovalResponse = self.query(querier, req)?;
        Ok(res)
    }
}
