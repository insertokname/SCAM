# S.C.A.M.

A nes emulator written in rust

Live wasm version here: https://scam.insertokname.com

**LLM usage:**

- `scamu` was fully done by me using the nes dev wiki, tricness, and olcnes as a refference
- `scami` was partially written by ai
- `scamm` was fully written using agentic swarms

Controls:

| nes             | layout 1    | layout 2 |
| --------------- | ----------- | -------- |
| Directional Pad | WASD        | Arrows   |
| A Button        | J           | Z        |
| B Button        | K           | X        |
| Start           | Enter       | C        |
| Select          | Right Shift | V        |



### Requirements

- Install [Rust](https://rust-lang.org/learn/get-started/)

### Building

To build the project:

```bash
cargo build
```

### Building the wasm version

Requirements: 
- `trunk`: install instructions here: https://github.com/trunk-rs/trunk.git

To run the project run `trunk serve` for easy reloading or `trunk build` if you want a static output.



