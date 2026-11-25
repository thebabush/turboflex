//! c26: branching on another module's / crate's enum — `match`, `if let`,
//! `let else`, `while let`, `matches!`, and a `match` on a field (no local of
//! the enum type).

use crate::payload::Job;

pub struct Holder {
    pub job: Job,
}

pub fn match_on(j: &Job) -> i32 {
    match j {
        Job::Idle => 0,
        Job::Resize(n) => *n as i32,
        Job::Move { dx, dy } => dx + dy,
    }
}

pub fn if_let(j: &Job) -> u32 {
    if let Job::Resize(n) = j { *n } else { 0 }
}

pub fn let_else(j: &Job) -> u32 {
    let Job::Resize(n) = j else {
        return 0;
    };
    *n
}

pub fn while_let(jobs: &[Job]) -> u32 {
    let mut it = jobs.iter();
    let mut total = 0;
    while let Some(Job::Resize(n)) = it.next() {
        total += n;
    }
    total
}

pub fn macro_match(j: &Job) -> bool {
    matches!(j, Job::Idle)
}

pub fn field_match(h: &Holder) -> bool {
    match h.job {
        Job::Idle => true,
        Job::Resize(_) | Job::Move { .. } => false,
    }
}

pub fn cross_crate(l: &core_api::Level) -> u8 {
    match l {
        core_api::Level::Low => 0,
        core_api::Level::High(n) => *n,
    }
}
