# Benchmarking

For checking performance of runtime, we do some performance tests

## Test 1: Udp echo server

## Test 2: External communication

```
RUST_LOG=info cargo run --release --example benchmark_ext
```

We doing a test with a echo style from controller to inside worker. The controller will send a message to worker, and worker will send back the message to controller. And the result, we can send 1.5M messages (1500 bytes) per second, that is 1.5M * 1500 * 8 = 18Gbps, this is just enought for almost of application. The latency is 2.5ms, because of we doing in polling base, maybe it can improve by using interrupt base.

```
[2024-03-19T02:39:11Z INFO  benchmark_queue] received: 1499250 mps, avg wait: 2667 us
[2024-03-19T02:39:12Z INFO  benchmark_queue] received: 1576044 mps, avg wait: 2537 us
[2024-03-19T02:39:13Z INFO  benchmark_queue] received: 1648804 mps, avg wait: 2426 us
[2024-03-19T02:39:15Z INFO  benchmark_queue] received: 1537279 mps, avg wait: 2603 us
[2024-03-19T02:39:16Z INFO  benchmark_queue] received: 1560062 mps, avg wait: 2564 us
[2024-03-19T02:39:17Z INFO  benchmark_queue] received: 1644736 mps, avg wait: 2431 us
```