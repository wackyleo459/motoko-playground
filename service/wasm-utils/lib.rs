use ic_cdk::export::candid::{CandidType, Deserialize};
use serde_bytes::ByteBuf;

mod instrumentation;
mod remove_cycles;
mod utils;

#[derive(CandidType, Deserialize)]
struct Config {
    profiling: bool,
    remove_cycles_add: bool,
    limit_stable_memory_page: Option<u32>,
}

#[ic_cdk_macros::query]
fn transform(wasm: ByteBuf, config: Config) -> ByteBuf {
    let mut m = walrus::Module::from_buffer(&wasm).unwrap();
    if config.profiling {
        instrumentation::instrument(&mut m);
    }
    if config.remove_cycles_add {
        remove_cycles::replace_cycles_add_with_drop(&mut m);
    }
    if let Some(page) = config.limit_stable_memory_page {
        remove_cycles::limit_stable_memory_page(&mut m, page);
    }
    let wasm = m.emit_wasm();
    ByteBuf::from(wasm)
}
