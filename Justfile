sim:
    cargo flamegraph || flamegraph --perfdata perf.data
    google-chrome flamegraph.svg
 
