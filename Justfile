sim:
    cargo flamegraph || flamegraph --perfdata perf.data
    google-chrome flamegraph.svg
 
hyperfine:
    cargo build --profile maxspeed --features no_tracing
    hyperfine ./target/maxspeed/lsof

coz: 
    mv profile.coz profile.coz.old
    cargo build --bin coz --profile coz --features coz
    coz run --- ./target/coz/lsof >/dev/null 2>&1

flamegraph:
    cargo flamegraph --bin lsof --freq 100000
    flamegraph --perfdata perf.data
    google-chrome flamegraph.svg
