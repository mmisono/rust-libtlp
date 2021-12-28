rust-libtlp
===========

Rust version of [LibTLP](https://github.com/NetTLP/libtlp/).

## Status
- [x] DMA Read
- [ ] DMA Write
- [ ] Messaging API
- [ ] Callback API
- [ ] PCIe Configuration API

## Usage

In Cargo.toml,

```
[dependencies]
libtlp = { git = "https://github.com/mmisono/rust-libtlp" }
```

## Examples
```shell
cargo run --example dma_read -- \
--bdf 01:00.0 --local 192.168.20.3 --remote 192.168.20.1 \
--address 0x100000 --size 32
```

## License
Dual-licensed under Apache-2.0 or MIT.

-----

Originally LibTLP is developed by [Ryo Nakamura](https://github.com/upa) and [Yohei Kuga](https://github.com/sora).
See [haeena.dev/nettlp/](https://haeena.dev/nettlp/) for the netlp information.

