//go:build !linux

package main

import "errors"

// testOpenFiles leaves descriptor auditing to Linux; temporary defaults and side-effect hooks are portable.
func testOpenFiles() (map[string]string, error) {
	return map[string]string{}, nil
}

// runGuardedTests reports that the syscall guard is unavailable here, so the suite runs in this process.
func runGuardedTests(_ []string) (int, bool) {
	return 0, false
}

// installWriteGuard fails because the guard is never started off Linux, so a child cannot ask for it.
func installWriteGuard() error {
	return errors.New("the syscall guard is available on linux only")
}
