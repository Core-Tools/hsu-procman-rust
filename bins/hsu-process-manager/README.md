# HSU Process Manager - Rust Binary

Process manager executable for the HSU Process Management.

## Quick Start

```bash
# Run with example config
cargo run -- --config config-example.yaml --debug

# Build release
cargo build --release

# Run with options
./target/release/hsu-process-manager --config config.yaml --port 50055
```

## CLI Options

```
Options:
  -c, --config <FILE>          Configuration file path (YAML)
  -d, --debug                  Enable debug logging
  -p, --port <PORT>           Port to listen on (overrides config)
      --run-duration <SECS>   Run for N seconds (testing)
  -h, --help                   Print help
  -V, --version                Print version
```

## Configuration

See `config-example.yaml` for a complete configuration example.

The YAML format is compatible with the Go implementation.

## Documentation

**📚 Complete documentation:** [`docs/v3/architecture/process-management/`](../../../../docs/v3/architecture/process-management/)

- **[Architecture Overview](../../../../docs/v3/architecture/process-management/README.md)**
- **[Rust Implementation Guide](../../../../docs/v3/architecture/process-management/implementation-rust.md)**
- **[Configuration Reference](../../../../docs/v3/architecture/process-management/README.md#configuration)**

## Development Status

🚧 **Under Active Development**

- ✅ Foundation complete (state machine, lifecycle, config)
- 🚧 Health checks, monitoring, logging in progress
- 📋 Advanced features planned

See [implementation guide](../../../../docs/v3/architecture/process-management/implementation-rust.md) for detailed status.

## Related

- **[Go Implementation](../../../go/)** - Production-ready reference
- **[Process Management Docs](../../../../docs/v3/architecture/process-management/)** - Architecture & design

---

*Part of HSU Core - Modular System Development*
