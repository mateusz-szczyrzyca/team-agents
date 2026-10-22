#!/usr/bin/env bats
# Unit tests of the instance table in `agent status` (multi-instance registry file bus/instances.json).

load helpers

# write_registry stores the registry fixture: one active instance (pi5) and one expired one (mac).
write_registry() {
  mkdir -p "$AGENTS_HOME/bus"
  cat >"$AGENTS_HOME/bus/instances.json" <<'EOF'
{
  "version": 1,
  "instances": {
    "pi5": {
      "peer": "token:pi5",
      "lease": {"version": 1, "lease_token": "x", "generation": 2, "expires_at": "2099-01-01 00:00:00"},
      "sequence": 4,
      "manifest": {
        "instance_name": "pi5",
        "detected": {"os": "linux", "arch": "arm64"},
        "declared": ["build:c", "hw:rp2350-probe"],
        "current_state": {"slots": {"total": 4, "free": 3}, "tasks": {"t1": {"task_id": "t1"}}}
      }
    },
    "mac": {
      "peer": "token:mac",
      "lease": {"version": 1, "lease_token": "y", "generation": 1, "expires_at": "2000-01-01 00:00:00"},
      "sequence": 9,
      "manifest": {
        "instance_name": "mac",
        "detected": {"os": "darwin", "arch": "arm64"},
        "declared": [],
        "current_state": {"slots": {"total": 2, "free": 0}}
      }
    }
  }
}
EOF
}

@test "agent status lists the recorded instances with lease, platform, slots, tasks and capabilities" {
  write_registry
  run agent status
  [ "$status" -eq 0 ]
  [[ $output == *"INSTANCE"*"LEASE"*"PLATFORM"*"CAPABILITIES"* ]]
  [[ $output == *"mac"*"expired"*"darwin/arm64"*"0/2"*"-"* ]]
  [[ $output == *"pi5"*"active"*"linux/arm64"*"3/4"*"build:c, hw:rp2350-probe"* ]]
  # rows are sorted by instance name: mac before pi5
  [[ $output == *"mac"*"pi5"* ]]
}

@test "agent status on a single-instance installation shows no instance table" {
  run agent status
  [ "$status" -eq 0 ]
  [[ $output != *"INSTANCE"* ]]
}

@test "agent status says when the registry file is missing but the instance is configured" {
  run env INSTANCE_NAME=pi5 agent status
  [ "$status" -eq 0 ]
  [[ $output == *"no instances recorded yet"* ]]
}
