package main

import (
	"io"
	"os"
	"syscall"
	"unsafe"
)

// writerIsTerminal queries a file's terminal settings; unsupported writers and ioctl failures mean false.
func writerIsTerminal(writer io.Writer) bool {
	file, ok := writer.(*os.File)
	if !ok {
		return false
	}
	// settings is the fixed-size buffer required by the Linux terminal query.
	var settings syscall.Termios
	_, _, errno := syscall.Syscall(syscall.SYS_IOCTL, file.Fd(), syscall.TCGETS, uintptr(unsafe.Pointer(&settings))) //nolint:gosec // G103: TCGETS writes only this fixed-size termios buffer.
	return errno == 0
}
