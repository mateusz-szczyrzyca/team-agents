# Developer tasks. `make test` runs everything that needs no agent accounts.
.PHONY: test unit-go unit-bats lint integration build generate check-generated

test: unit-go unit-bats

unit-go:
	cd agentbus && go vet ./... && go test ./...
	cd live && go vet ./... && go test ./...

unit-bats:
	bats tests/unit

lint: check-generated
	bash -n bin/agent install.sh tests/integration/run.sh
	@if command -v shellcheck >/dev/null; then shellcheck -S warning bin/agent install.sh tests/integration/run.sh examples/notify/*.sh; else echo "shellcheck not installed — skipped"; fi
	cd agentbus && test -z "$$(gofmt -l .)"
	cd live && test -z "$$(gofmt -l .)"
	cd tools/doccheck && go build -o ../../build/doccheck .
	build/doccheck agentbus/*.go live/*.go

# Real agent CLIs, real accounts (costs usage). See tests/integration/run.sh.
integration:
	AGENT_IT=1 tests/integration/run.sh

build:
	cd agentbus && go build -o ../build/agentbus .
	cd live && go build -o ../build/agentlive .

# Regenerates agentbus/errors_gen.go from agentbus/errors.json.
generate:
	cd agentbus && go generate ./...

# Fails when errors_gen.go is out of date with errors.json.
check-generated:
	mkdir -p build
	cd tools/errgen && go run . -spec ../../agentbus/errors.json -out ../../build/errors_gen.go
	cmp -s build/errors_gen.go agentbus/errors_gen.go || { echo "agentbus/errors_gen.go is out of date: run make generate"; exit 1; }
