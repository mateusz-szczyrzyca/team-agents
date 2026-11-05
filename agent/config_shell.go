package main

import "context"

// loadConfig evaluates executable Bash settings before applying startup defaults.
func (a *App) loadConfig(ctx context.Context, req ConfigRequest) (Config, *LoadConfigError) {
	cfg, err := a.loadShellConfig(ctx, req)
	if err != nil {
		return Config{}, err
	}
	return applyConfigDefaults(cfg, req.Context), nil
}

// loadShellConfig isolates configuration output from its typed NUL snapshot.
//
//nolint:unparam // RED stub returns zero values so behavior tests compile before implementation.
func (a *App) loadShellConfig(_ context.Context, _ ConfigRequest) (Config, *LoadConfigError) {
	return newConfig(), nil
}

// decodeConfigNUL validates declaration and element records from the allowlisted channel.
//
//nolint:unparam // RED stub returns zero values so malformed protocol assertions can fail.
func decodeConfigNUL(_ []byte) (Config, *LoadConfigError) {
	return newConfig(), nil
}

// configNames lists startup settings whose declarations may cross the Bash boundary.
func configNames() []string {
	return nil
}
