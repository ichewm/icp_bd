mod clients;
mod common;

use std::borrow::BorrowMut;
use std::cell::{RefCell, Cell};
use crate::clients::dip20::Dip20;
use crate::clients::sonic::Sonic;
use crate::clients::xtc::{XTCBurnPayload, XTC};
use crate::clients::nns_cycles_minting::{NNS_Cycle_Minting, IcpXdrConversionRateCertifiedResponse, IcpXdrConversionRate};
use crate::clients::black_hole::{BlackHole, CanisterStatusArg0};
use crate::common::guards::controller_guard;
use crate::common::types::{Currency, LimitOrder, MarketOrder, Order, OrderDirective, TargetPrice, OrganizeName, OrganizeOwner, MemberInfo, CanisterInfo};

use std::collections::BTreeMap;

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



type Members = RefCell<BTreeMap<Principal, RefCell<MemberInfo>>>;
type Canisters = RefCell<BTreeMap<Principal, RefCell<CanisterInfo>>>;

type OrganizesToMembers = BTreeMap<OrganizeName, Members>;  // 组织映射组员
type OrganizesToCanisters = BTreeMap<OrganizeName, Canisters>;  // 组织映射罐
type OrganizesToOwner = BTreeMap<OrganizeName, RefCell<OrganizeOwner>>;  // 组织映射所有者

// 组织所有者 下组织及用户输出结构
type OrganizationOwnerMemberOutput = Vec<OrganizesToMembers>;


// 存储结构
thread_local!{
    static ORGANIZES_TO_MEMBERS:RefCell<OrganizesToMembers> = RefCell::default();
    static ORGANIZES_TO_CANISTERS:RefCell<OrganizesToCanisters> = RefCell::default();
    static ORGANIZES_TO_OWNER:RefCell<OrganizesToOwner> = RefCell::default();
}

// 创建组织
#[update]
pub async fn create_organize(organize_name: String) -> String {
    let organize_owner = ic_cdk::api::caller();
    // 判断当前 组织名是否已经存在 【组织名不可再罐内重复】
    ORGANIZES_TO_OWNER.with(|organizes_to_owner|{
        if organizes_to_owner.borrow().contains_key(&organize_name){
            String::from("organize name already exists")  // organize name 已经存在
        } else {
            organizes_to_owner.borrow_mut().insert(organize_name, RefCell::new(organize_owner));
            String::from("organize name created successfully")  // organize name 创建成功
        }
    })

}

// 转让组织所有权
#[update]
pub async fn transfer_organization_ownership(organize_name: String, new_owner: Principal) -> String {
    let old_owner = ic_cdk::api::caller();
    ORGANIZES_TO_OWNER.with(|organizes_to_owner|{
        if organizes_to_owner.borrow().contains_key(&organize_name){
            if organizes_to_owner.borrow().get(&organize_name).unwrap() == &RefCell::new(old_owner) {
                organizes_to_owner.borrow_mut().insert(organize_name, RefCell::new(new_owner));
                String::from("Transfer Organization Ownership Success")  // 转让组织所有权 成功
            } else {
                String::from("Non-organization owner, cannot perform transfer")  // 非组织所有人，无法执行转让
            }
        } else {
            String::from("organization does not exist")  // 组织不存在
        }
    })
}

// 删除组织
#[update]
pub async fn disband_the_organization(organize_name: String) -> String {
    let organize_owner = ic_cdk::api::caller();
    ORGANIZES_TO_OWNER.with(|organizes_to_owner|{
        if organizes_to_owner.borrow().contains_key(&organize_name){
            if organizes_to_owner.borrow().get(&organize_name).unwrap() == &RefCell::new(organize_owner) {
                organizes_to_owner.borrow_mut().remove_entry(&organize_name);
                String::from("Organization Disbanded Successfully")  // 组织解散成功
            } else {
                String::from("Non-organization owner, cannot perform dissolution")  // 非组织所有人，无法执行解散
            }
        } else {
            String::from("organization does not exist")  // 组织不存在
        }
    })
}

// 组织所有人 向组织 添加成员
#[update]
pub async fn organization_owner_add_members_to_organization(member_id: Principal, member_name: String, organize_name: String) -> String {
    let requester_id = ic_cdk::api::caller();
    ORGANIZES_TO_OWNER.with(|organizes_to_owner|{
        // 组织必须存在
        if !organizes_to_owner.borrow().contains_key(&organize_name){
            String::from("organization does not exist")  // 组织不存在
        }

        // 操作人必须是 owner
        if organizes_to_owner.borrow().get(&organize_name).unwrap() != &RefCell::new(requester_id){
            String::from("Non-organization owners cannot add members")  // 非组织所有者不可添加成员
        }

        ORGANIZES_TO_MEMBERS.with(|organizes_to_members|{
            // 检查组织是否存在不存在就新增
            if organizes_to_members.borrow().get(&organize_name).is_some(){
                // 检查这个成员是否存在
                if organizes_to_members.borrow().get(&organize_name).unwrap().borrow().get(&member_id).is_some(){
                    String::from("The member already exists in this organization")  // 该成员已经存在于这个组织
                } else {
                    // 成员不存在 新增成员
                    organizes_to_members.borrow_mut().get(&organize_name).unwrap().borrow_mut().insert(
                        member_id, 
                        RefCell::new(
                            MemberInfo{
                                nickname: member_name,
                                instime: Cell::new(ic_cdk::api::time())
                            }
                        ));
                    String::from("added successfully")  // 新增成功
                }
            } else {
                // 组织如果不存在就新增组织并添加成员

                // 创建成员结构
                let mut members: Members= BTreeMap::new().into();
                members.borrow_mut().insert(
                    member_id, 
                    RefCell::new(MemberInfo{
                        nickname: member_name,
                        instime: Cell::new(ic_cdk::api::time())
                    })
                );
                // 插入 组织
                organizes_to_members.borrow_mut().insert(
                    organize_name,
                    members
                );
                String::from("Organization member added successfully")  // 组织成员新增成功
            }
        })
    })
}


// 组织 所有人 减掉 组织成员
#[update]
pub async fn organization_owner_minus_organization_members(member_id: Principal, organize_name: String) -> String {
    let requester_id = ic_cdk::api::caller();
    ORGANIZES_TO_OWNER.with(|organizes_to_owner|{
        // 组织必须存在
        if !organizes_to_owner.borrow().contains_key(&organize_name){
            String::from("organization does not exist")  // 组织不存在
        }
        // 操作人必须是 owner
        if organizes_to_owner.borrow().get(&organize_name).unwrap() != &RefCell::new(requester_id){
            String::from("Non-organization owners cannot add members")  // 非组织所有者不可添加成员
        }
        ORGANIZES_TO_MEMBERS.with(|organizes_to_members|{
            // 检查组织是否存在不存在就新增
            if organizes_to_members.borrow().get(&organize_name).is_some(){
                // 检查这个成员是否存在 存在就删除成员
                if organizes_to_members.borrow().get(&organize_name).unwrap().borrow().get(&member_id).is_some(){
                    organizes_to_members.borrow_mut().get(&organize_name).unwrap().borrow_mut().remove(&member_id);
                    String::from("The member has been removed from the organization")  // 该成员已在组织中删除
                } else {
                    // 成员不存在 新增成员
                    String::from("The member does not exist in the organization")  // 该成员不存在于组织中
                }
            } else {
                // 组织如果不存在说明还没有添加过成员 直接返回成员不存在
                String::from("The member does not exist in the organization")  // 该成员不存在于组织中
            }
        })
    })
}


// 组织所有者查询自己名下组织及组织下的用户
#[query]
pub async fn the_organization_owner_queries_the_organization_under_his_own_name_and_the_users_under_the_organization() -> OrganizationOwnerMemberOutput {
    let requester_id = ic_cdk::api::caller();
    // 找到这个人的所有组织
    ORGANIZES_TO_OWNER.with(|organizes_to_owner|{
        // 创建一个存储 组织名的 向量
        let mut organizes:Vec<String> = Vec::new();
        // 创建一个输出 结构
        let mut organization_owner_member_output = OrganizationOwnerMemberOutput::new();

        for (organize_name, owner_id) in organizes_to_owner.borrow_mut().iter(){
            if *owner_id == RefCell::new(requester_id) {
                // let f = organize_name.to_string();
                organizes.push(organize_name.to_string());
            }
        }
        ORGANIZES_TO_MEMBERS.with(|organizes_to_members|{
            // // 循环 组织名向量 获取所有组织下的所有成员
            for organize_name in organizes {
                match organizes_to_members.borrow().get(&organize_name) {
                    Some(member_info) => {
                        let mut o_t_m = OrganizesToMembers::new();
                        o_t_m.insert(organize_name, member_info.clone());
                        organization_owner_member_output.push(o_t_m);
                    },
                    None => (),
                }
            }
        });
        organization_owner_member_output

    })
}


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
