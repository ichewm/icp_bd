use std::cell::Cell;

use candid::Principal;
use ic_cdk::export::candid::{CandidType, Deserialize, Nat};

// 组织名
pub type OrganizeName = String;

// 组织所有者
pub type OrganizeOwner = Principal;


// 成员
#[derive(CandidType, Deserialize, Clone)]
pub struct MemberInfo {
    pub nickname: String,  // 别称
    pub instime: Cell<u64>,  // 插入时间 此为管理员插入
}

// 罐信息
#[derive(CandidType, Deserialize, Clone)]
pub struct  CanisterInfo {
    pub nickname: String, // 罐别称
    pub instime:Cell<u64>,  // 罐插入时间
    pub updtime:Cell<u64>,  // 上次更新Cycles时间
    pub cycles_balance: Cell<u64>,  // 罐余额
    pub time_interval: Cell<u64>,  // 轮训时间间隔
    pub cycles_minimum: Cell<u64>,  // 最低Cycles
    pub cycles_highest:Cell<u64>,  // 最高Cycles
}

// 公共罐信息
#[derive(CandidType, Deserialize, Clone)]
pub struct  PubilcCanisterInfo {
    pub updtime:Cell<u64>,  // 公共上次更新Cycles时间
    pub cycles_balance: Cell<u64>,  // 罐余额
    pub time_interval: Cell<u64>,  // 轮训时间间隔
    pub cycles_minimum: Cell<u64>,  // 公共最低Cycles
    pub cycles_highest:Cell<u64>,  // 公共最高Cycles
}

// 用户充值ICP记录
#[derive(CandidType, Deserialize, Clone, Copy)]
pub struct UserRechargeICPRecordInfo {
    pub recharge_time: u64,  // 充值时间
    pub recharge_amount: u64,  // 充值金额
}

// 罐映射组织信息
#[derive(CandidType, Deserialize, Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct CanisterMappingOrganizationInfo {
    pub organize_name: String,  // 组织名
    pub min_cycles: Cell<u64>,  // 最小罐循环
}




#[derive(CandidType, Deserialize)]
pub enum Order {
    Market(MarketOrder),
    Limit(LimitOrder),
}

#[derive(CandidType)]
pub enum DepositErr {
    BalanceLow,
    TransferFailure,
}

#[derive(CandidType, Deserialize, Clone)]
pub struct MarketOrder {
    pub give_currency: Currency,
    pub take_currency: Currency,
    pub directive: OrderDirective,
}

#[derive(CandidType, Deserialize, Clone)]
pub struct LimitOrder {
    pub target_price_condition: TargetPrice,
    pub market_order: MarketOrder,
}

#[derive(CandidType, Deserialize, Clone)]
pub enum TargetPrice {
    MoreThan(f64),
    LessThan(f64),
}

#[derive(CandidType, Deserialize, Clone)]
pub enum OrderDirective {
    GiveExact(Nat),
    TakeExact(Nat),
}

#[derive(CandidType, Deserialize, Clone)]
pub enum Currency {
    ICP,
    XTC,
    WICP,
    SONIC,
    NnsCyclesMinting,
    BlackHole,
}


#[derive(CandidType, Deserialize, Clone, PartialEq)]
pub enum Opts {
    ADD,
    UPDATE,
    DELETE,
}