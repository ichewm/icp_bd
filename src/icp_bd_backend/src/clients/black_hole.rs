use async_trait::async_trait;
use ic_cdk::api::call::CallResult;
use ic_cdk::call;
use ic_cdk::export::candid::{CandidType, Deserialize, Principal, Nat};

#[derive(CandidType, Deserialize)]
pub struct DefiniteCanisterSettings {
    pub freezing_threshold: candid::Nat,
    pub controllers: Vec<candid::Principal>,
    pub memory_allocation: candid::Nat,
    pub compute_allocation: candid::Nat,
}

#[derive(CandidType, Deserialize)]
pub struct CanisterStatusArg0 { 
    pub canister_id: Principal 
}

#[derive(CandidType, Deserialize)]
pub enum canister_status_status { stopped, stopping, running }

#[derive(CandidType, Deserialize)]
pub struct CanisterStatus {
  pub status: canister_status_status,
  pub memory_size: candid::Nat,
  pub cycles: candid::Nat,
  pub settings: DefiniteCanisterSettings,
  pub module_hash: Option<Vec<u8>>,
}


#[async_trait]
pub trait BlackHole {
    async fn canister_status(&self, canister_id: CanisterStatusArg0) -> CallResult<(CanisterStatus,)>;
}

#[async_trait]
impl BlackHole for Principal {
    async fn canister_status(&self, canister_id: CanisterStatusArg0) -> CallResult<(CanisterStatus,)> {
        call(*self, "canister_status", (canister_id,)).await
    }
}