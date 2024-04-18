// Copyright © Aptos Foundation
// SPDX-License-Identifier: Apache-2.0

use aptos_experimental_active_state_set::pipeline::{ExecutionMode, Pipeline, PipelineConfig};
use aptos_logger::info;
use std::env;
use tempfile::TempDir;

#[cfg(unix)]
#[global_allocator]
static ALLOC: jemallocator::Jemalloc = jemallocator::Jemalloc;

pub fn main() {
    // set the default log level to debug
    aptos_logger::Logger::new().init();
    env::set_var("RUST_LOG", "info");
    let path = TempDir::new().unwrap().path().to_str().unwrap().to_string();
    info!("Pipeline data stored at {}", path);
    let config = PipelineConfig::new(1, 3, path, ExecutionMode::AST);
    let mut pipeline = Pipeline::new(config);
    pipeline.run();
}
