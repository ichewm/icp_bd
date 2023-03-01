mod clients;
mod common;

use std::borrow::BorrowMut;
use crate::clients::dip20::Dip20;
use crate::clients::sonic::Sonic;
use crate::clients::xtc::{XTCBurnPayload, XTC};
use crate::clients::nns_cycles_minting::{NNS_Cycle_Minting, IcpXdrConversionRateCertifiedResponse, IcpXdrConversionRate};
use crate::clients::black_hole::{BlackHole, CanisterStatusArg0};
use crate::common::guards::controller_guard;
use crate::common::types::{Currency, LimitOrder, MarketOrder, Order, OrderDirective, TargetPrice};
use bigdecimal::num_bigint::{BigInt, ToBigInt};
use bigdecimal::num_traits::Pow;
use bigdecimal::{BigDecimal, FromPrimitive, ToPrimitive};
use ic_cdk::api::{canister_balance, time};
use ic_cdk::export::candid::{export_service, CandidType, Deserialize, Int, Nat, Principal};
use ic_cdk::id;
use ic_cdk::storage::{stable_restore, stable_save};
use ic_cdk_macros::{heartbeat, init, post_upgrade, pre_upgrade, query, update};
use ic_cron::implement_cron;
use ic_cron::types::{Iterations, SchedulingOptions, TaskId};
use ic_ledger_types::{AccountIdentifier, Subaccount, DEFAULT_SUBACCOUNT, AccountBalanceArgs, TransferArgs, Memo, Tokens, BlockIndex, TransferResult};


async fn get_swap_price_internal(give_currency: Currency, take_currency: Currency) -> BigDecimal {
    let state = get_state();
    let give_token = token_id_by_currency(give_currency);
    let take_token = token_id_by_currency(take_currency);

    let (pair_opt,) = Sonic::get_pair(&state.sonic_swap_canister, give_token, take_token)
        .await
        .expect("Unable to fetch pair at Sonic");

    let pair = pair_opt.unwrap();

    let give_reserve = BigDecimal::from(pair.reserve0.0.to_bigint().unwrap());
    let take_reserve = BigDecimal::from(pair.reserve1.0.to_bigint().unwrap());

    give_reserve / take_reserve
}


#[query]
pub fn my_cycles_balance() -> u64 {
    canister_balance()
}

#[query]
pub fn my_canister_config(token: Currency) -> Principal {
    token_id_by_currency(token.clone())
}

#[query]
pub fn ic_time() -> u64 {
    ic_cdk::api::time()
}


#[query]
fn select_canister_account_id() -> String {
    let canister_id = ic_cdk::api::id();
    let id = ic_cdk::api::caller();  // 当前请求用户的唯一标识
    AccountIdentifier::new(
        &canister_id,
        &Subaccount::from(id)
    ).to_string()
}


#[update]
pub async fn icp_balance(account_id: Principal) -> u64 {
    let state = get_state();

    let ac_id = AccountIdentifier::new(&account_id, &DEFAULT_SUBACCOUNT);
    let balance_args = AccountBalanceArgs { account: ac_id };

    let balance = ic_ledger_types::account_balance(state.icp_canister, balance_args)
        .await.expect("no balance");
    balance.e8s()
}


#[update]
pub async fn get_cycles_rate() -> f64 {
    let start = get_state();
    let get_icp_xdr_conversion_rate_response =  NNS_Cycle_Minting::get_icp_xdr_conversion_rate(&start.nns_cycles_minting_canister).await.expect("Server error").0;
    get_icp_xdr_conversion_rate_response.data.xdr_permyriad_per_icp.to_f64().unwrap()/10000f64
}

// 使用 blackhole 查询周期余额
// 需要执行以下命令
// dfx canister --network=ic update-settings --add-controller e3mmv-5qaaa-aaaah-aadma-cai iwzcr-cqaaa-aaaan-qc6sa-cai
#[update]
pub async fn use_black_hole_cycles_balance(canister_id:Principal) -> Nat {
    let start = get_state();
    let canister_id_status = BlackHole::canister_status(&start.black_hole_canister, CanisterStatusArg0{canister_id: canister_id}).await.expect("Server error").0;
    canister_id_status.cycles
}



#[update]
pub async fn get_swap_price(give_currency: Currency, take_currency: Currency) -> f64 {
    let give_token = token_id_by_currency(give_currency.clone());
    let take_token = token_id_by_currency(take_currency.clone());

    let price_bd = get_swap_price_internal(give_currency, take_currency).await;

    let (give_token_decimals,) = Dip20::decimals(&give_token)
        .await
        .expect("Unable to fetch give_token decimals");

    let (take_token_decimals,) = Dip20::decimals(&take_token)
        .await
        .expect("Unable to fetch take_token decimals");

    let decimals_dif =
        give_token_decimals.to_i32().unwrap() - take_token_decimals.to_i32().unwrap();

    let decimals_modifier = 10f64.pow(decimals_dif);

    price_bd.to_f64().unwrap() * decimals_modifier
}

fn token_id_by_currency(currency: Currency) -> Principal {
    let state = get_state();

    match currency {
        Currency::XTC => state.xtc_canister,
        Currency::WICP => state.wicp_canister,
        Currency::ICP => state.icp_canister,
        Currency::SONIC => state.sonic_swap_canister,
        Currency::NnsCyclesMinting => state.nns_cycles_minting_canister,
        Currency::BlackHole => state.black_hole_canister,
    }
}

// -------------------- STATE ---------------------

#[derive(CandidType, Deserialize, Clone, Copy)]
pub struct State {
    pub icp_canister: Principal,
    pub xtc_canister: Principal,
    pub wicp_canister: Principal,
    pub sonic_swap_canister: Principal,
    pub nns_cycles_minting_canister: Principal,
    pub black_hole_canister: Principal,
}

pub static mut STATE: Option<State> = None;

pub fn get_state() -> &'static State {
    unsafe { STATE.as_ref().unwrap() }
}

#[init]
pub fn init() {
    unsafe {
        STATE = Some(State {
            icp_canister: Principal::from_text("ryjl3-tyaaa-aaaaa-aaaba-cai").unwrap(),
            xtc_canister: Principal::from_text("aanaa-xaaaa-aaaah-aaeiq-cai").unwrap(),
            wicp_canister: Principal::from_text("utozz-siaaa-aaaam-qaaxq-cai").unwrap(),
            sonic_swap_canister: Principal::from_text("3xwpq-ziaaa-aaaah-qcn4a-cai").unwrap(),
            nns_cycles_minting_canister: Principal::from_text("rkp4c-7iaaa-aaaaa-aaaca-cai").unwrap(),
            black_hole_canister: Principal::from_text("e3mmv-5qaaa-aaaah-aadma-cai").unwrap(),
        })
    }
}

implement_cron!();
