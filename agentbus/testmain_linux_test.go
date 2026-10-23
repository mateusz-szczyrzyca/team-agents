//go:build linux

package main

import (
	"errors"
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"runtime"
	"strings"
	"syscall"
	"unsafe"
)

// Constants of the syscall guard. The numbers and the register layout are the linux/amd64 ABI, the only
// architecture the guard runs on; other architectures keep the descriptor audit only.
const (
	sysOpen    = 2    // open(2) on amd64
	sysOpenat  = 257  // openat(2) on amd64
	sysSeccomp = 317  // seccomp(2) on amd64, not defined by package syscall
	atFdcwd    = -100 // AT_FDCWD: relative paths start at the working directory

	ptraceEventSeccomp  = 7       // PTRACE_EVENT_SECCOMP: a seccomp filter returned SECCOMP_RET_TRACE
	ptraceOTraceSeccomp = 1 << 7  // PTRACE_O_TRACESECCOMP, not defined by package syscall
	ptraceOExitKill     = 1 << 20 // PTRACE_O_EXITKILL: tracees are killed when the tracer exits

	prSetNoNewPrivs        = 38         // PR_SET_NO_NEW_PRIVS: lets an unprivileged process install a filter
	seccompSetModeFilter   = 1          // SECCOMP_SET_MODE_FILTER
	seccompFilterFlagTsync = 1          // SECCOMP_FILTER_FLAG_TSYNC: the filter applies to all threads
	seccompRetKill         = 0x80000000 // SECCOMP_RET_KILL_PROCESS: verdict for a foreign architecture
	seccompRetTrace        = 0x7ff00000 // SECCOMP_RET_TRACE with data 0: report the call to the tracer
	seccompRetAllow        = 0x7fff0000 // SECCOMP_RET_ALLOW
	seccompOffNr           = 0          // offset of the system call number in struct seccomp_data
	seccompOffArch         = 4          // offset of the architecture in struct seccomp_data
	auditArchX8664         = 0xc000003e // AUDIT_ARCH_X86_64

	bpfLdWAbs = 0x20 // BPF_LD|BPF_W|BPF_ABS: load a 32-bit word of the input
	bpfJeqK   = 0x15 // BPF_JMP|BPF_JEQ|BPF_K: jump by the offsets when the word equals K
	bpfRetK   = 0x06 // BPF_RET|BPF_K: return the verdict K
)

// writeOpenFlags are the open(2) flags that can modify a file or create one.
const writeOpenFlags = uint64(syscall.O_WRONLY | syscall.O_RDWR | syscall.O_CREAT | syscall.O_TRUNC | syscall.O_APPEND)

// writeGuard decides which open calls are writes into a protected home.
type writeGuard struct {
	homes []string // resolved directories no test may write into
	deny  bool     // make a blocked open fail with EACCES instead of letting it write
}

// writeFilter is the seccomp program of the traced child: open and openat stop at the tracer, every other system
// call runs without a stop, so the tests keep their speed.
var writeFilter = []syscall.SockFilter{
	{Code: bpfLdWAbs, K: seccompOffArch},
	{Code: bpfJeqK, Jt: 1, K: auditArchX8664},
	{Code: bpfRetK, K: seccompRetKill},
	{Code: bpfLdWAbs, K: seccompOffNr},
	{Code: bpfJeqK, Jt: 2, K: sysOpen},
	{Code: bpfJeqK, Jt: 1, K: sysOpenat},
	{Code: bpfRetK, K: seccompRetAllow},
	{Code: bpfRetK, K: seccompRetTrace},
}

// testOpenFiles inventories this process only; concurrent agents may write their own real logs safely.
func testOpenFiles() (map[string]string, error) {
	entries, err := os.ReadDir("/proc/self/fd")
	if err != nil {
		return nil, err
	}
	files := make(map[string]string)
	for _, entry := range entries {
		path, err := os.Readlink(filepath.Join("/proc/self/fd", entry.Name()))
		if os.IsNotExist(err) {
			continue // ReadDir's own descriptor may already have closed
		}
		if err != nil {
			return nil, err
		}
		files[entry.Name()] = path
	}
	return files, nil
}

// installWriteGuard runs in the traced child before any test: it installs the seccomp filter that makes open and
// openat stop at the tracer. Threads and child processes created later inherit the filter.
func installWriteGuard() error {
	if _, _, errno := syscall.Syscall(syscall.SYS_PRCTL, prSetNoNewPrivs, 1, 0); errno != 0 {
		return fmt.Errorf("prctl(PR_SET_NO_NEW_PRIVS): %w", errno)
	}
	prog := syscall.SockFprog{Len: uint16(len(writeFilter)), Filter: &writeFilter[0]}                                        //nolint:gosec // G115: the filter has 8 instructions
	_, _, errno := syscall.Syscall(sysSeccomp, seccompSetModeFilter, seccompFilterFlagTsync, uintptr(unsafe.Pointer(&prog))) //nolint:gosec // G103: seccomp(2) reads the program through this pointer
	if errno != 0 {
		return fmt.Errorf("seccomp(SECCOMP_SET_MODE_FILTER): %w", errno)
	}
	return nil
}

// runGuardedTests runs the whole suite as a traced child and reports whether it did so; the child and
// platforms without the guard return false and the suite runs in this process.
func runGuardedTests(homes []string) (int, bool) {
	if os.Getenv(guardChildEnv) != "" {
		return 0, false
	}
	if runtime.GOARCH != "amd64" {
		fmt.Fprintln(os.Stderr, "test isolation: syscall guard supports linux/amd64 only; the descriptor audit remains active")
		return 0, false
	}
	guarded := resolveHomes(homes)
	probeDir, err := os.MkdirTemp("", "agentbus-guard-")
	if err != nil {
		fmt.Fprintln(os.Stderr, "test isolation: cannot create guard self-check directory:", err)
		return 1, true
	}
	defer func() {
		if err := os.RemoveAll(probeDir); err != nil {
			fmt.Fprintln(os.Stderr, "test isolation: cannot remove guard self-check directory:", err)
		}
	}()
	// Self-check: a file written and closed in a protected directory must still be reported.
	probeEnv := append(os.Environ(), guardChildEnv+"=1", guardProbeEnv+"="+probeDir)
	found, probeExit, err := traceWrites(writeGuard{homes: resolveHomes([]string{probeDir}), deny: false}, probeEnv, nil)
	if err != nil {
		fmt.Fprintln(os.Stderr, "test isolation: guard self-check failed:", err)
		return 1, true
	}
	if probeExit != 0 {
		fmt.Fprintf(os.Stderr, "test isolation: guard self-check probe exited with status %d\n", probeExit)
		return 1, true
	}
	if found == 0 {
		fmt.Fprintln(os.Stderr, "test isolation: guard self-check: a closed write to a protected home was not detected")
		return 1, true
	}
	// Real run: writes into the real agents home are denied and counted.
	env := append(os.Environ(), guardChildEnv+"=1", guardResultEnv+"=detected")
	violations, code, err := traceWrites(writeGuard{homes: guarded, deny: true}, env, os.Args[1:])
	if err != nil {
		fmt.Fprintln(os.Stderr, "test isolation: syscall guard failed:", err)
		return 1, true
	}
	if violations != 0 {
		fmt.Fprintf(os.Stderr, "test isolation: %d writes to the real agents home by the test process tree\n", violations)
		return 1, true
	}
	return code, true
}

// resolveHomes returns the symbolic-link-free form of each non-empty home, so that resolved paths compare equal.
func resolveHomes(homes []string) []string {
	var resolved []string
	for _, home := range homes {
		if home == "" {
			continue
		}
		path, err := filepath.EvalSymlinks(home)
		if err != nil {
			path = filepath.Clean(home) // missing directories cannot be written through symbolic links
		}
		resolved = append(resolved, path)
	}
	return resolved
}

// traceWrites runs this test binary with args and env under ptrace and counts the open calls that write into
// guard.homes. The child installs the filter in installWriteGuard; the tracer follows every thread and child
// process and checks each call when it is made, so a file that is closed again before the end of the run is
// still counted. It returns the violation count and the exit status of the traced binary.
func traceWrites(guard writeGuard, env []string, args []string) (int, int, error) {
	// ptrace requests must come from the thread that started the tracee, so the whole run stays on this thread.
	runtime.LockOSThread()
	defer runtime.UnlockOSThread()
	exe, err := os.Executable()
	if err != nil {
		return 0, 0, err
	}
	cmd := exec.Command(exe, args...) //nolint:gosec // G204: runs this test binary itself, never external input
	cmd.Env = env
	cmd.Stdout = os.Stdout
	cmd.Stderr = os.Stderr
	cmd.SysProcAttr = &syscall.SysProcAttr{Ptrace: true}
	if err := cmd.Start(); err != nil {
		return 0, 0, err
	}
	pid := cmd.Process.Pid
	var status syscall.WaitStatus
	if _, err := syscall.Wait4(pid, &status, 0, nil); err != nil {
		return 0, 0, err
	}
	opts := syscall.PTRACE_O_TRACECLONE | syscall.PTRACE_O_TRACEFORK | syscall.PTRACE_O_TRACEVFORK |
		syscall.PTRACE_O_TRACEEXEC | ptraceOTraceSeccomp | ptraceOExitKill
	if err := syscall.PtraceSetOptions(pid, opts); err != nil {
		return 0, 0, err
	}
	if err := syscall.PtraceCont(pid, 0); err != nil {
		return 0, 0, err
	}
	violations, exitCode := 0, 0
	for {
		tid, err := syscall.Wait4(-1, &status, syscall.WALL, nil)
		if errors.Is(err, syscall.EINTR) {
			continue
		}
		if errors.Is(err, syscall.ECHILD) {
			break
		}
		if err != nil {
			return violations, exitCode, err
		}
		if status.Exited() || status.Signaled() {
			if tid == pid {
				exitCode = 1 // killed by a signal counts as a failure
				if status.Exited() {
					exitCode = status.ExitStatus()
				}
			}
			continue
		}
		sig := status.StopSignal()
		deliver := 0
		switch {
		case sig == syscall.SIGTRAP && status.TrapCause() == ptraceEventSeccomp:
			n, err := guard.check(tid)
			if err != nil {
				return violations, exitCode, err
			}
			violations += n
		case sig != syscall.SIGTRAP && sig != syscall.SIGSTOP:
			deliver = int(sig) // real signals reach the test; other SIGTRAP and SIGSTOP stops are ptrace's own events
		}
		if err := syscall.PtraceCont(tid, deliver); err != nil && !errors.Is(err, syscall.ESRCH) {
			return violations, exitCode, err
		}
	}
	if err := cmd.Process.Release(); err != nil {
		return violations, exitCode, err
	}
	return violations, exitCode, nil
}

// check inspects the seccomp stop of thread tid and returns 1 when it is an open of a write into a protected home.
func (g writeGuard) check(tid int) (int, error) {
	var regs syscall.PtraceRegs
	if err := syscall.PtraceGetRegs(tid, &regs); err != nil {
		if errors.Is(err, syscall.ESRCH) {
			return 0, nil // the thread exited after the stop
		}
		return 0, err
	}
	dirfd := int64(atFdcwd)
	var pathAddr, flags uint64
	switch regs.Orig_rax {
	case sysOpen:
		pathAddr, flags = regs.Rdi, regs.Rsi
	case sysOpenat:
		dirfd, pathAddr, flags = int64(int32(regs.Rdi)), regs.Rsi, regs.Rdx //nolint:gosec // G115: dirfd is a C int, so the upper 32 bits of the register are not meaningful
	default:
		return 0, nil
	}
	if flags&writeOpenFlags == 0 || pathAddr == 0 {
		return 0, nil
	}
	raw, err := readString(tid, uintptr(pathAddr))
	if err != nil {
		return 0, err
	}
	path, err := resolvePath(tid, dirfd, raw)
	if err != nil {
		return 0, err
	}
	if !g.protects(path) {
		return 0, nil
	}
	if g.deny {
		// -1 cancels the system call; rax is then the result the call reports: -EACCES in two's complement.
		regs.Orig_rax = ^uint64(0)
		regs.Rax = ^(uint64(syscall.EACCES) - 1)
		if err := syscall.PtraceSetRegs(tid, &regs); err != nil {
			return 0, err
		}
	}
	fmt.Fprintf(os.Stderr, "test isolation: write to a protected home by thread %d: %s\n", tid, path)
	return 1, nil
}

// protects reports whether path is a guarded home or lies inside one; a sibling with a shared name prefix does not match.
func (g writeGuard) protects(path string) bool {
	for _, home := range g.homes {
		if path == home || strings.HasPrefix(path, home+string(filepath.Separator)) {
			return true
		}
	}
	return false
}

// readString copies the NUL-terminated string at addr from the memory of thread pid.
func readString(pid int, addr uintptr) (string, error) {
	var b strings.Builder
	var word [8]byte
	for off := uintptr(0); off < 4096; off += 8 {
		n, err := syscall.PtracePeekData(pid, addr+off, word[:])
		if err != nil {
			return "", err
		}
		for _, c := range word[:n] {
			if c == 0 {
				return b.String(), nil
			}
			b.WriteByte(c)
		}
	}
	return "", errors.New("path argument longer than 4096 bytes")
}

// resolvePath returns the absolute path of an open call, with symbolic links resolved when the file or its
// directory exists. A path that does not resolve is returned cleaned: the open call fails for the same reason.
func resolvePath(pid int, dirfd int64, path string) (string, error) {
	if !filepath.IsAbs(path) {
		base := fmt.Sprintf("/proc/%d/cwd", pid)
		if dirfd != atFdcwd {
			base = fmt.Sprintf("/proc/%d/fd/%d", pid, dirfd)
		}
		dir, err := os.Readlink(base)
		if err != nil {
			return "", err
		}
		path = filepath.Join(dir, path)
	}
	if resolved, err := filepath.EvalSymlinks(path); err == nil {
		return resolved, nil
	}
	if parent, err := filepath.EvalSymlinks(filepath.Dir(path)); err == nil {
		return filepath.Join(parent, filepath.Base(path)), nil // a file that does not exist yet
	}
	return filepath.Clean(path), nil
}
