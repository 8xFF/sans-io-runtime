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
| [x]  | [ ] | [x]   | [ ]       | Control | Controller to worker communication |
| [ ]  | [ ] | [ ]   | [ ]       | Control | Controller to task communication   |
| [x]  | [ ] | [x]   | [ ]       | Control | Workers status monitoring          |
| [x]  | [ ] | [x]   | [ ]       | I/O     | Udp                                |
| [ ]  | [ ] | [ ]   | [ ]       | I/O     | Tcp                                |
| [ ]  | [ ] | [ ]   | [ ]       | I/O     | Rpc                                |
| [x]  | [ ] | [x]   | [ ]       | Backend | mio                                |
| [ ]  | [ ] | [ ]   | [ ]       | Backend | io_uring                           |
| [ ]  | [ ] | [ ]   | [ ]       | Backend | af_xdp                             |
| [x]  | [ ] | [x]   | [ ]       | Example | Udp echo server                    |
| [x]  | [ ] | [x]   | [ ]       | Example | Udp echo client                    |
| [x]  | [ ] | [x]   | [ ]       | Example | Simple Whip/Whep server            |

## Design

![Design](./docs/design.excalidraw.png)
