//go:build !linux

package main

// testOpenFiles leaves descriptor auditing to Linux; temporary defaults and side-effect hooks are portable.
func testOpenFiles() (map[string]string, error) {
	return map[string]string{}, nil
}
