mod clients;
mod common;

// use rand::Rng;
use std::borrow::BorrowMut;
use std::cell::{RefCell, Cell};
use std::ops::IndexMut;
use crate::clients::dip20::Dip20;
use crate::clients::sonic::Sonic;
use crate::clients::xtc::{XTCBurnPayload, XTC};
use crate::clients::nns_cycles_minting::{NNS_Cycle_Minting, IcpXdrConversionRateCertifiedResponse, IcpXdrConversionRate};
use crate::clients::black_hole::{BlackHole, CanisterStatusArg0};
use crate::common::guards::controller_guard;
use crate::common::types::{Currency, LimitOrder, MarketOrder, Order, OrderDirective, TargetPrice, OrganizeName, OrganizeOwner, MemberInfo, CanisterInfo, PubilcCanisterInfo, CanisterMappingOrganizationInfo, Opts, UserRechargeICPRecordInfo};

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

// 公共罐结构 所有组织下的罐都映射到这个 BT 中, 此中只记录 罐余额， 轮训时间间隔取 所有组织罐中最低的 最低Cycles取最低的，最高Cycles取最高的
type PublicCanisters = BTreeMap<Principal, PubilcCanisterInfo>;
// 记录这个罐都被那个组织添加了，以备在罐余额不足时直接命中组织进而找到成员，组织排序方式按照最低Cycles进行排序,可以找到最低设置Cycle用户
// Vec 有排序方法 CanistersInfo.sort_by(|op, m| m.min_cycles.cmp(&op.min_cycles));
type CanistersToOrganizes = BTreeMap<Principal, RefCell<Vec<CanisterMappingOrganizationInfo>>>;
// 存储用户充值记录结构  只能查到余额 使用了怎么办
type UserRechargeRecordStructure = BTreeMap<Principal, RefCell<Vec<UserRechargeICPRecordInfo>>>;



// 组织所有者 下组织及用户输出结构
type OrganizationOwnerMemberOutput = Vec<OrganizesToMembers>;
// 组织所有者 下组织及罐输出结构
type OrganizationOwnerCanisterOutput = Vec<OrganizesToCanisters>;


// 存储结构
thread_local!{
    static ORGANIZES_TO_MEMBERS:RefCell<OrganizesToMembers> = RefCell::default();
    static ORGANIZES_TO_CANISTERS:RefCell<OrganizesToCanisters> = RefCell::default();
    static ORGANIZES_TO_OWNER:RefCell<OrganizesToOwner> = RefCell::default();
    static PUBLIC_CANISTERS:RefCell<PublicCanisters> = RefCell::default();
    static CANISTERS_TO_ORGANIZES:RefCell<CanistersToOrganizes> = RefCell::default();
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
                delete_synchronously(&organize_name);
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
           return String::from("organization does not exist");  // 组织不存在
        };

        // 操作人必须是 owner
        if organizes_to_owner.borrow().get(&organize_name).unwrap() != &RefCell::new(requester_id){
            return String::from("Non-organization owners cannot add members");  // 非组织所有者不可添加成员
        };

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
                let members: Members= BTreeMap::new().into();
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
        });
    String::from("added successfully")
    })
}


// 组织 所有人 减掉 组织成员
#[update]
pub async fn organization_owner_minus_organization_members(member_id: Principal, organize_name: String) -> String {
    let requester_id = ic_cdk::api::caller();
    ORGANIZES_TO_OWNER.with(|organizes_to_owner|{
        // 组织必须存在
        if !organizes_to_owner.borrow().contains_key(&organize_name){
            return String::from("organization does not exist");  // 组织不存在
        }
        // 操作人必须是 owner
        if organizes_to_owner.borrow().get(&organize_name).unwrap() != &RefCell::new(requester_id){
            return String::from("Non-organization owners cannot add members");  // 非组织所有者不可添加成员
        };
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
                    None => {
                        let mut o_t_m = OrganizesToMembers::new();
                        let members: Members= BTreeMap::new().into();
                        o_t_m.insert(organize_name, members);
                        organization_owner_member_output.push(o_t_m);
                    },
                }
            }
        });
        organization_owner_member_output

    })
}


// 组织所有人向组织添加新罐
#[update]
pub async fn the_organization_owner_adds_a_new_jar_to_the_organization(organize_name:String, canister_name: String, canister_id: Principal, time_interval: u64, cycles_minimum: u64, cycles_highest: u64) -> String {
    let requester_id = ic_cdk::api::caller();
    // 正式环境使用 测试没有问题
    // let cycle_balance = use_black_hole_cycles_balance(canister_id).await.0.to_u64().unwrap();
    // 测试环境使用直接赋值
    let cycle_balance = generate_random_numbers().await;
    ORGANIZES_TO_OWNER.with(|organizes_to_owner|{
        // 组织必须存在
        if !organizes_to_owner.borrow().contains_key(&organize_name){
           return String::from("organization does not exist");  // 组织不存在
        };

        // 操作人必须是 owner
        if organizes_to_owner.borrow().get(&organize_name).unwrap() != &RefCell::new(requester_id){
            return String::from("Non-organization owners cannot add members");  // 非组织所有者不可添加成员
        };
        ORGANIZES_TO_CANISTERS.with(|organizes_to_canisters|{
            // 检查组织是否存在不存在就新增
            if organizes_to_canisters.borrow().get(&organize_name).is_some(){
                // 检查这个罐是否存在
                if organizes_to_canisters.borrow().get(&organize_name).unwrap().borrow().get(&canister_id).is_some(){
                    String::from("The canister already exists in this organization")  // 该罐已经存在于这个组织
                } else {
                    // 罐不存在 新增罐
                    organizes_to_canisters.borrow_mut().get(&organize_name).unwrap().borrow_mut().insert(
                        canister_id, 
                        RefCell::new(
                            CanisterInfo{
                                nickname: canister_name,
                                instime: Cell::new(ic_cdk::api::time()),
                                updtime: Cell::new(ic_cdk::api::time()),
                                cycles_balance: Cell::new(cycle_balance),
                                time_interval: Cell::new(time_interval),
                                cycles_minimum: Cell::new(cycles_minimum),
                                cycles_highest: Cell::new(cycles_highest),
                            }
                        ));
                    // 为公共罐结构增加罐
                    add_or_update_public_canisters(
                        canister_id, 
                        ic_cdk::api::time(), 
                        cycle_balance, 
                        time_interval, 
                        cycles_minimum, 
                        cycles_highest
                    );
                    // 记录该罐被那些组织收录逻辑
                    canister_mapping_organization_deal_with(
                        Opts::ADD, 
                        canister_id, 
                        organize_name, 
                        cycles_minimum);
                    String::from("added successfully")  // 新增成功
                }
            } else {
                // 组织如果不存在就新增组织并添加罐

                // 创建罐结构
                let canisters: Canisters= BTreeMap::new().into();
                canisters.borrow_mut().insert(
                    canister_id, 
                    RefCell::new(CanisterInfo{
                        nickname: canister_name,
                        instime: Cell::new(ic_cdk::api::time()),
                        updtime: Cell::new(ic_cdk::api::time()),
                        cycles_balance: Cell::new(cycle_balance),
                        time_interval: Cell::new(time_interval),
                        cycles_minimum: Cell::new(cycles_minimum),
                        cycles_highest: Cell::new(cycles_highest),
                    })
                );
                // 插入 组织
                organizes_to_canisters.borrow_mut().insert(
                    organize_name.clone(),
                    canisters
                );
                // 为公共罐结构增加罐
                add_or_update_public_canisters(
                    canister_id, 
                    ic_cdk::api::time(), 
                    cycle_balance, 
                    time_interval, 
                    cycles_minimum, 
                    cycles_highest
                );
                // 记录该罐被那些组织收录逻辑
                canister_mapping_organization_deal_with(
                    Opts::ADD, 
                    canister_id, 
                    organize_name.clone(), 
                    cycles_minimum);
                String::from("Organization canister added successfully")  // 组织罐新增成功
            }
        });
    String::from("added successfully")
    })
}


// 组织所有人 删除罐
#[update]
pub async fn organization_owner_delete_jar(organize_name:String, canister_id: Principal) -> String {
    let requester_id = ic_cdk::api::caller();
    ORGANIZES_TO_OWNER.with(|organizes_to_owner|{
        // 组织必须存在
        if !organizes_to_owner.borrow().contains_key(&organize_name){
            return String::from("organization does not exist");  // 组织不存在
        }
        // 操作人必须是 owner
        if organizes_to_owner.borrow().get(&organize_name).unwrap() != &RefCell::new(requester_id){
            return String::from("Non-organization owners cannot add canister");  // 非组织所有者不可删除罐
        };
        ORGANIZES_TO_CANISTERS.with(|organizes_to_canisters|{
            if organizes_to_canisters.borrow().get(&organize_name).is_some(){
                // 检查这个罐是否存在 存在就删除罐
                if organizes_to_canisters.borrow().get(&organize_name).unwrap().borrow().get(&canister_id).is_some(){
                    organizes_to_canisters.borrow_mut().get(&organize_name).unwrap().borrow_mut().remove(&canister_id);
                    // 记录该罐被那些组织删除逻辑
                    canister_mapping_organization_deal_with(
                        Opts::DELETE, 
                        canister_id, 
                        organize_name, 
                        0u64);
                    String::from("The canister has been removed from the organization")  // 该罐已在组织中删除
                } else {
                    String::from("The canister does not exist in the organization")  // 该罐不存在于组织中
                }
            } else {
                // 组织如果不存在说明还没有添加过罐 直接返回罐不存在
                String::from("The canister does not exist in the organization")  // 该罐不存在于组织中
            }
        })
    })
}


// 组织所有人 修改罐
#[update]
pub async fn organization_owner_modify_jar(organize_name: String, canister_id:Principal, time_interval:u64, cycles_minimum:u64, cycles_highest:u64) -> String {
    let requester_id = ic_cdk::api::caller();
    // let cycle_balance = use_black_hole_cycles_balance(canister_id).await.0.to_u64().unwrap();
    let cycle_balance = generate_random_numbers().await;
    ORGANIZES_TO_OWNER.with(|organizes_to_owner|{
        // 组织必须存在
        if !organizes_to_owner.borrow().contains_key(&organize_name){
           return String::from("organization does not exist");  // 组织不存在
        };

        // 操作人必须是 owner
        if organizes_to_owner.borrow().get(&organize_name).unwrap() != &RefCell::new(requester_id){
            return String::from("Non-organization owners cannot add members");  // 非组织所有者不可添加成员
        };

        ORGANIZES_TO_CANISTERS.with(|organizes_to_canisters|{
            if organizes_to_canisters.borrow().get(&organize_name).is_some(){
                // 检查这个罐是否存在
                if organizes_to_canisters.borrow().get(&organize_name).unwrap().borrow().get(&canister_id).is_some(){
                    // 如果罐存在就更新字段
                    organizes_to_canisters.borrow().get(&organize_name).unwrap().borrow().get(&canister_id).unwrap().borrow_mut().time_interval.set(time_interval);
                    organizes_to_canisters.borrow().get(&organize_name).unwrap().borrow().get(&canister_id).unwrap().borrow_mut().cycles_minimum.set(cycles_minimum);
                    organizes_to_canisters.borrow().get(&organize_name).unwrap().borrow().get(&canister_id).unwrap().borrow_mut().cycles_highest.set(cycles_highest);
                    organizes_to_canisters.borrow().get(&organize_name).unwrap().borrow().get(&canister_id).unwrap().borrow_mut().cycles_balance.set(cycle_balance);
                     // 为公共罐结构增加罐
                     add_or_update_public_canisters(
                        canister_id, 
                        ic_cdk::api::time(), 
                        cycle_balance, 
                        time_interval, 
                        cycles_minimum, 
                        cycles_highest
                    );
                    // 记录该罐被那些组织修改逻辑
                    canister_mapping_organization_deal_with(
                        Opts::UPDATE, 
                        canister_id, 
                        organize_name, 
                        cycles_minimum);
                    String::from("Canister details updated successfully")  // canister 详情更新成功

                } else {
                    // 罐不存在 返回罐不存在
                    String::from("The canister does not exist under this organization") 
                }
            } else {
                // 组织如果不存就返回 罐不存在
                String::from("The canister does not exist under this organization")  // 组织罐新增成功
            }
        })
    })
}


// 组织所有人 查询自己名下组织及组织下的罐
#[query]
pub async fn organization_owner_query_the_organization_under_his_name_and_the_tanks_under_the_organization() -> OrganizationOwnerCanisterOutput {
    let requester_id = ic_cdk::api::caller();
    // 找到这个人的所有组织
    ORGANIZES_TO_OWNER.with(|organizes_to_owner|{
        // 创建一个存储 组织名的 向量
        let mut organizes:Vec<String> = Vec::new();
        // 创建一个输出 结构
        let mut organization_owner_canister_output = OrganizationOwnerCanisterOutput::new();

        for (organize_name, owner_id) in organizes_to_owner.borrow_mut().iter(){
            if *owner_id == RefCell::new(requester_id) {
                // let f = organize_name.to_string();
                organizes.push(organize_name.to_string());
            }
        }

        ORGANIZES_TO_CANISTERS.with(|organizes_to_canisters|{
            // // 循环 组织名向量 获取所有组织下的所有罐
            for organize_name in organizes {
                match organizes_to_canisters.borrow().get(&organize_name) {
                    Some(canister_info) => {
                        let mut o_t_m = OrganizesToCanisters::new();
                        o_t_m.insert(organize_name, canister_info.clone());
                        organization_owner_canister_output.push(o_t_m);
                    },
                    None => {
                        let mut o_t_m = OrganizesToCanisters::new();
                        let canisters: Canisters= BTreeMap::new().into();
                        o_t_m.insert(organize_name, canisters);
                        organization_owner_canister_output.push(o_t_m);
                    },
                }
            }
        });
        organization_owner_canister_output
    })
}


// 测试期间方法  
// 查询公共映射罐结构
#[query]
pub async fn query_the_structure_of_the_public_rotation_training_tank() -> PublicCanisters {
    PUBLIC_CANISTERS.with(|public_canisters|{
        public_canisters.borrow_mut().clone()
    })
}

// 按照 cycles 排序 组织
#[query]
pub async fn organize_according_to_cycles_sorting(canister_id: Principal) -> Vec<CanisterMappingOrganizationInfo> {
    CANISTERS_TO_ORGANIZES.with(|canisters_to_organizes|{
        canisters_to_organizes.borrow_mut().get(&canister_id).unwrap().borrow_mut().sort_by(|op, m| m.min_cycles.cmp(&op.min_cycles));
        canisters_to_organizes.borrow_mut().get(&canister_id).unwrap().borrow_mut().clone()
    })
}

// 生成随机假定的 cycles
#[query]
pub async fn generate_random_numbers () -> u64 {
    // let mut rng = rand::thread_rng();
    // rng.gen::<u64>()
    999u64
}


// 私有方法 
// 同步删除
fn delete_synchronously (organize_name:&String) {
    // 删除组织的同时删除组织成员
    ORGANIZES_TO_MEMBERS.with(|organizes_to_members|{
        match organizes_to_members.borrow().get(organize_name) {
            Some(member_info) => {
                organizes_to_members.borrow_mut().remove(organize_name);
            },
            None => (),
        }
    });
    // 删除组织的同时删除组织罐
    ORGANIZES_TO_CANISTERS.with(|organizes_to_canisters|{
        match organizes_to_canisters.borrow().get(organize_name) {
            Some(canister_info) => {
                organizes_to_canisters.borrow_mut().remove(organize_name);
            },
            None => (),
        }
    })
}

// 公共罐映射 添加和修改
fn add_or_update_public_canisters (canister_id: Principal,updtime: u64, cycles_balance: u64, time_interval: u64, cycles_minimum: u64, cycles_highest: u64) {
    // 公共罐映射 添加和修改
    PUBLIC_CANISTERS.with(|public_canisters|{
        // 这个罐存在
        if public_canisters.borrow().get(&canister_id).is_some(){
            // 获取公共罐的 time_interval cycles_minimum cycles_highest
            if public_canisters.borrow().get(&canister_id).unwrap().time_interval > Cell::new(time_interval) {
                // 找最小的轮训时间间隔
                public_canisters.borrow().get(&canister_id).unwrap().time_interval.set(time_interval)
            }
            if public_canisters.borrow().get(&canister_id).unwrap().cycles_minimum > Cell::new(cycles_minimum) {
                // 找最低限度的 Cycles
                public_canisters.borrow().get(&canister_id).unwrap().cycles_minimum.set(cycles_minimum)
            }
            if public_canisters.borrow().get(&canister_id).unwrap().cycles_highest < Cell::new(cycles_highest) {
                // 找最高限度的 Cycles
                public_canisters.borrow().get(&canister_id).unwrap().cycles_highest.set(cycles_highest)
            }
            // 因为每触发一次此函数都会重新读取一次罐的 Cycles
            // 故同步更新 公共 updtime  cycles_balance
            public_canisters.borrow().get(&canister_id).unwrap().updtime.set(updtime);
            public_canisters.borrow().get(&canister_id).unwrap().cycles_balance.set(cycles_balance);
        } else {
            // 这个罐不存在 新增罐
            public_canisters.borrow_mut().insert(
                canister_id,
                PubilcCanisterInfo{
                    updtime:Cell::new(updtime),
                    cycles_balance:Cell::new(cycles_balance),
                    time_interval:Cell::new(time_interval),
                    cycles_minimum:Cell::new(cycles_minimum),
                    cycles_highest:Cell::new(cycles_highest),
                }
            );
        }
    })
}


// 罐映射组织排序 操作 [新增/修改/删除]
fn canister_mapping_organization_deal_with(opt: Opts, canister_id:Principal, organize_name: String, min_cycles: u64) {
    // 罐组织映射结构操作有三种
    
    CANISTERS_TO_ORGANIZES.with(|canisters_to_organizes|{
        // 当这个罐 组织 映射不存在时 对 且 opt 为 ADD时 对 Vec 进行初始化新增
        if opt == Opts::ADD {
            // 罐组织不存在是进行初始化新增
            if canisters_to_organizes.borrow_mut().get(&canister_id).is_none(){
                let mut v_c_m_z_i = Vec::new();
                v_c_m_z_i.push(
                    CanisterMappingOrganizationInfo{
                        organize_name: organize_name,
                        min_cycles:Cell::new(min_cycles),
                    }
                );
                canisters_to_organizes.borrow_mut().insert(
                    canister_id, 
                    RefCell::new(v_c_m_z_i),
                );
            } else {
                // 罐组织存在 进行 push 新增
                canisters_to_organizes.borrow_mut().get(&canister_id).unwrap().borrow_mut().push(
                    CanisterMappingOrganizationInfo {
                        organize_name: organize_name,
                        min_cycles: Cell::new(min_cycles),
                    }
                );
            }
        } else if opt == Opts::UPDATE {
            // 罐组织存在修改 min_cycles
            canisters_to_organizes.borrow_mut().get(&canister_id).unwrap().borrow_mut().iter_mut().find(|cmoi|{cmoi.organize_name == organize_name}).unwrap().borrow_mut().min_cycles.set(min_cycles);
            // target.borrow_mut().min_cycles.set(min_cycles);

        } else {
            // 罐组织存在 进行删除
            // if let Some(target)= canisters_to_organizes.borrow_mut().get(&canister_id).unwrap().borrow_mut().iter_mut().position(|cmoi|{cmoi.organize_name == organize_name}) {
            //     // target.borrow_mut().organize_name.remove(index);
            //     canisters_to_organizes.borrow_mut().get(&canister_id).unwrap().borrow_mut().remove(target);
            // } else {
            //     ();
            // }
            let mut target= canisters_to_organizes.borrow_mut().get(&canister_id).unwrap().borrow_mut().iter().position(|cmoi|{cmoi.organize_name == organize_name}).unwrap();
            canisters_to_organizes.borrow_mut().get(&canister_id).unwrap().borrow_mut().remove(target);
            
        }
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

// 生成属于这个罐的用户id
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
