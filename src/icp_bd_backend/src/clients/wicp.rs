use async_trait::async_trait;
use ic_cdk::api::call::{CallResult};
use ic_cdk::call;
use ic_cdk::export::candid::{CandidType, Deserialize, Principal, Nat};
use ic_ledger_types::Subaccount;


#[derive(CandidType, Deserialize, Debug)]
pub enum WICPError {
    InsufficientAllowance,
    InsufficientBalance,
    ErrorOperationStyle,
    Unauthorized,
    LedgerTrap,
    ErrorTo,
    Other,
    BlockUsed,
    AmountTooSmall,
}

pub type WICPResult = Result<Nat, WICPError>;

#[async_trait]
pub trait WICP {
    async fn mint(&self, to: Option<Subaccount>, block_index: u64) -> CallResult<(WICPResult,)>;
}

#[async_trait]
impl WICP for Principal {
    async fn mint(&self, to: Option<Subaccount>, block_index: u64) -> CallResult<(WICPResult,)> {
        call(*self, "mint", (to,  block_index)).await
    }

}
