use std::hint::black_box;

use lsof::*;
use tracing_subscriber::prelude::*;

fn main() {
    tracing_coz::TracingCozBridge::new().init();

    for _ in 0..100000 {
        let d = black_box(Data::lsof_all());
    }
}
