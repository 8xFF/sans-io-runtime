<p align="center">
 <a href="https://github.com/8xFF/sans-io-runtime/actions">
  <img src="https://github.com/8xFF/sans-io-runtime/actions/workflows/rust.yml/badge.svg?branch=main">
 </a>
 <a href="https://codecov.io/gh/8xff/sans-io-runtime">
  <img src="https://codecov.io/gh/8xff/sans-io-runtime/branch/main/graph/badge.svg">
 </a>
 <a href="https://deps.rs/repo/github/8xff/sans-io-runtime">
  <img src="https://deps.rs/repo/github/8xff/sans-io-runtime/status.svg">
 </a>
 <a href="https://crates.io/crates/sans-io-runtime">
  <img src="https://img.shields.io/crates/v/sans-io-runtime.svg">
 </a>
 <a href="https://docs.rs/sans-io-runtime">
  <img src="https://docs.rs/sans-io-runtime/badge.svg">
 </a>
 <a href="https://github.com/8xFF/sans-io-runtime/blob/main/LICENSE">
  <img src="https://img.shields.io/badge/license-MIT-blue" alt="License: MIT">
 </a>
 <a href="https://discord.gg/tJ6dxBRk">
  <img src="https://img.shields.io/discord/1173844241542287482?logo=discord" alt="Discord">
 </a>
</p>

# SANS-I/O runtime (Working in progress)

(This module is in very early stage of development. It is not ready for production use.)

This is a simple, lightweight, and fast runtime for the SansIo mechanism.

## Goal

The goal of this project is to provide a simple, lightweight, and fast runtime for the SansIo mechanism. The runtime should be able to run on any platform with variables network library like: mio, io_uring, af_xdp.

## How it works

Controller will spawn some threads and each thread will run a worker. The workers

## Features

| Impl | C/I | Works | Benchmark | Group   | Description                        |
| ---- | --- | ----- | --------- | ------- | ---------------------------------- |
| [x]  | [ ] | [x]   | [ ]       | Control | Cross tasks communication          |
| [x]  | [ ] | [x]   | [x]       | Control | Controller to worker communication |
| [x]  | [ ] | [x]   | [ ]       | Control | Controller to task communication   |
| [x]  | [ ] | [x]   | [ ]       | Control | Workers status monitoring          |
| [x]  | [ ] | [x]   | [ ]       | I/O     | Udp                                |
| [x]  | [ ] | [x]   | [ ]       | I/O     | Tun/Tap                            |
| [ ]  | [ ] | [ ]   | [ ]       | I/O     | Tcp                                |
| [ ]  | [ ] | [ ]   | [ ]       | I/O     | Rpc                                |
| [x]  | [ ] | [x]   | [ ]       | Backend | mio                                |
| [x]  | [ ] | [x]   | [ ]       | Backend | raw poll                           |
| [x]  | [ ] | [x]   | [ ]       | Backend | polling                            |
| [ ]  | [ ] | [ ]   | [ ]       | Backend | io_uring                           |
| [ ]  | [ ] | [ ]   | [ ]       | Backend | af_xdp                             |
| [x]  | [ ] | [x]   | [ ]       | Example | Udp echo server                    |
| [x]  | [ ] | [x]   | [ ]       | Example | Udp echo client                    |
| [x]  | [ ] | [x]   | [ ]       | Example | Simple Whip/Whep server            |

## [Benchmarking](./docs/benchmark.md)

- External communication can archive 1.5M messages (1500 bytes) per second, that is 1.5M * 1500 * 8 = 18Gbps, this is just enought for almost of application. The latency is 2.5ms, because of we doing in polling base, maybe it can improve by using interrupt base.

## Design

![Design](./docs/design.excalidraw.png)

### Single task

Bellow is state diagram of a single task.

```mermaid
stateDiagram
    [*] --> Created
    Created --> Waiting : attach to worker
    Waiting --> OnTick : timer fired
    OnTick --> Waiting : no output
    OnTick --> PopOutput : has output
    PopOutput --> PopOutput : has output
    PopOutput --> Waiting : no output
    Waiting --> OnInput : I/O, Bus
    OnInput --> Waiting : no output
    OnInput --> PopOutput : has output
```

The idea is in SAN/IO style, each task will reduce memory by create output immediately after input. We need to pop the output before we can receive the next input.

### Multi tasks

With idea of SAN/IO is we need to pop the output before we can receive the next input. This is a problem when we have multiple tasks. We need to have a way to control the order of the tasks.

```mermaid
stateDiagram
    [*] --> Created
    Created --> Waiting : attach groups to worker
    Waiting --> OnTick : timer fired
    OnTick --> OnTickSingleTask : next task
    OnTick --> Waiting : no task
    OnTickSingleTask --> OnTick : no output
    OnTickSingleTask --> PopCurrentTickTaskOutput : has output
    PopCurrentTickTaskOutput --> PopCurrentTickTaskOutput : has output
    PopCurrentTickTaskOutput --> OnTick : no output

    Waiting --> OnInput : I/O, Bus
    OnInput --> OnInputSingleTask : has task
    OnInputSingleTask --> Waiting : no output
    OnInputSingleTask --> PopCurrentInputTaskOutput : has output
    PopCurrentInputTaskOutput --> PopCurrentInputTaskOutput : has output
    PopCurrentInputTaskOutput --> Waiting : no output
```