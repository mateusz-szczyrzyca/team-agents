package main

import (
	"bytes"
	"context"
	"fmt"
	"os"
	"strings"
	"syscall"
	"testing"
	"unsafe"
)

// TestLoggerTerminalWarning verifies terminal detection and warning delivery with a real PTY.
func TestLoggerTerminalWarning(t *testing.T) {
	master, err := os.OpenFile("/dev/ptmx", os.O_RDWR|syscall.O_NOCTTY, 0)
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() {
		if err := master.Close(); err != nil {
			t.Error(err)
		}
	})
	// unlock permits opening this test's PTY slave.
	var unlock int32
	if _, _, errno := syscall.Syscall(syscall.SYS_IOCTL, master.Fd(), syscall.TIOCSPTLCK, uintptr(unsafe.Pointer(&unlock))); errno != 0 { //nolint:gosec // G103: Linux ioctl requires a pointer to this fixed-size int32 buffer.
		t.Fatal(errno)
	}
	// number identifies the slave allocated for this master.
	var number uint32
	if _, _, errno := syscall.Syscall(syscall.SYS_IOCTL, master.Fd(), syscall.TIOCGPTN, uintptr(unsafe.Pointer(&number))); errno != 0 { //nolint:gosec // G103: Linux ioctl writes only this fixed-size uint32 buffer.
		t.Fatal(errno)
	}
	slave, err := os.OpenFile(fmt.Sprintf("/dev/pts/%d", number), os.O_RDWR|syscall.O_NOCTTY, 0)
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() {
		if err := slave.Close(); err != nil {
			t.Error(err)
		}
	})
	app := newApp(nil, nil, nil)
	app.agentHome, app.Stderr = t.TempDir(), slave
	logger := app.newLogger("show")
	if !logger.Interactive {
		t.Fatal("stderr PTY was not detected as interactive")
	}
	logger.Journal = false // Keep this delivery test independent of the host journal.
	if err := logger.Log(context.Background(), LevelWarn, LogInvListFailed); err != nil {
		t.Fatal(err)
	}
	if err := syscall.SetNonblock(int(master.Fd()), true); err != nil {
		t.Fatal(err)
	}
	output := make([]byte, 1024)
	n, err := syscall.Read(int(master.Fd()), output)
	if n < 0 {
		n = 0
	}
	if err != nil || !strings.Contains(string(output[:n]), "inv_list_failed") {
		t.Fatalf("terminal warning: %q, %v", output[:n], err)
	}
	app.Stderr = &bytes.Buffer{}
	if app.newLogger("show").Interactive {
		t.Fatal("nonterminal buffer detected as interactive")
	}
}
