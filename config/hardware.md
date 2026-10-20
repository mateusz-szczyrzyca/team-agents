# Environment notes (read by the agents)

Describe the shared resources the agents may use and how to address them. Every shared resource gets a
name; agents call `reserve <name>` before using it and `release <name>` right after. Delete what does not
apply and add your own sections.

## Example: debug probes for microcontrollers

### probe-a
- Debug probe serial number: `<SERIAL>` (find it with `probe-rs list`)
- probe-rs selector: `--probe <VID>:<PID>:<SERIAL>`
- Target chip: `<chip name for probe-rs>`
- Target board UART: `/dev/serial/by-id/<...>`

## Example: test hosts

### test-vps
- `ssh root@test-vps.example` (key-based) — disposable, may be broken and reinstalled
- Never touch production hosts.

## Rules
- Several devices may share a USB VID:PID — always select them by serial number.
- Reserve before use, release right after.
