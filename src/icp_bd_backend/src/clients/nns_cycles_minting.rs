use async_trait::async_trait;
use ic_cdk::api::call::{call_with_payment, CallResult};
use ic_cdk::call;
use ic_cdk::export::candid::{CandidType, Deserialize, Nat, Principal};

#[derive(CandidType, Deserialize)]
pub struct IcpXdrConversionRateCertifiedResponse {
  certificate: Vec<u8>,
  data: IcpXdrConversionRate,
  hash_tree: Vec<u8>,
}


#[derive(CandidType, Deserialize)]
pub struct IcpXdrConversionRate {
  xdr_permyriad_per_icp: u64,
  timestamp_seconds: u64,
}


#[async_trait]
pub trait NNS_Cycle_Minting {
    async fn get_icp_xdr_conversion_rate(&self) -> CallResult<(IcpXdrConversionRateCertifiedResponse,)>;
}

#[async_trait]
impl NNS_Cycle_Minting for Principal {
    async fn get_icp_xdr_conversion_rate(&self,) -> CallResult<(IcpXdrConversionRateCertifiedResponse,)> {
        ic_cdk::call(*self, "get_icp_xdr_conversion_rate", ()).await
      }
}
