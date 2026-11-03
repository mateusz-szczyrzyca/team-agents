package main

import (
	"errors"
	"fmt"
	"testing"
)

// TestConcreteErrorsPreserveClassification checks concrete types and underlying failures.
func TestConcreteErrorsPreserveClassification(t *testing.T) {
	// underlying models a failed standard-library operation.
	underlying := errors.New("operation failed")
	// cases cover each constructor, including failures without an underlying error.
	cases := []struct {
		name    string     // constructor being exercised
		failure classified // concrete lower-level error
		class   ErrorClass // expected category
		cause   string     // expected diagnostic cause
		wrapped bool       // whether underlying must be retained
	}{
		{"storage", storageErr(underlying, "write %s", "stdout"), ClassStorage, "Storage", true},
		{"value", valueErr("bad %s", "flag"), ClassInvalidArgument, "InvalidValue", false},
		{"wrapped value", valueWrap(underlying, "parse %s", "flag"), ClassInvalidArgument, "InvalidValue", true},
		{"process", processErr("helper", underlying, "run %s", "helper"), ClassProcess, "Process", true},
		{"process without underlying", processErr("helper", nil, "run helper"), ClassProcess, "Process", false},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			// The constructors return typed pointers; reject typed nil before calling methods.
			switch e := tc.failure.(type) {
			case *StorageError:
				if e == nil {
					t.Fatal("nil StorageError")
				}
			case *ValueError:
				if e == nil {
					t.Fatal("nil ValueError")
				}
			case *ProcessError:
				if e == nil {
					t.Fatal("nil ProcessError")
				}
				if e.Program != "helper" {
					t.Fatalf("program = %q", e.Program)
				}
			}
			if tc.failure.Class() != tc.class || tc.failure.CauseName() != tc.cause || tc.failure.Error() == "" {
				t.Fatalf("incorrect concrete failure: %v", tc.failure)
			}
			if !errors.Is(tc.failure, tc.class.Sentinel()) || errors.Is(tc.failure, underlying) != tc.wrapped {
				t.Fatal("lost sentinel or underlying error")
			}
			if class, cause := classOf(fmt.Errorf("outer: %w", tc.failure)); class != tc.class.String() || cause != tc.cause {
				t.Fatalf("classification = %q/%q", class, cause)
			}
		})
	}
}

// TestLowerErrorConversions checks exact operation causes, concrete chains and typed nil.
func TestLowerErrorConversions(t *testing.T) {
	// underlying models a failure retained across both conversion layers.
	underlying := errors.New("underlying")
	// storage must remain discoverable through the operation error.
	storage := storageErr(underlying, "write stdout")
	// operation represents conversion into the generated storage cause.
	operation := fromStorage(CommandStorage, storage)
	if operation == nil {
		t.Fatal("nil storage conversion")
	}
	// extracted verifies that the concrete low-level error survives conversion.
	var extracted *StorageError
	if operation.Cause != CommandStorage || operation.Class() != ClassStorage || !errors.Is(operation, underlying) || !errors.As(operation, &extracted) || extracted != storage {
		t.Fatal("storage conversion lost cause or chain")
	}
	// invalid must remain discoverable through an invalid-argument operation.
	invalid := valueWrap(underlying, "parse flag")
	// valueOperation represents conversion into the existing argument cause.
	valueOperation := fromValue(CommandInvalidArgument, invalid)
	if valueOperation == nil {
		t.Fatal("nil value conversion")
	}
	// valueExtracted verifies concrete value error identity.
	var valueExtracted *ValueError
	if valueOperation.Cause != CommandInvalidArgument || !errors.Is(valueOperation, ErrInvalidArgument) || !errors.Is(valueOperation, underlying) || !errors.As(valueOperation, &valueExtracted) || valueExtracted != invalid {
		t.Fatal("value conversion lost cause or chain")
	}
	if fromStorage(CommandStorage, nil) != nil || fromValue(CommandInvalidArgument, nil) != nil {
		t.Fatal("nil conversion became a failure")
	}
}

// TestLowerErrorInterfaces checks nil and non-nil conversions at error interface boundaries.
func TestLowerErrorInterfaces(t *testing.T) {
	// storage and value model successful lower-level operations.
	var storage *StorageError
	var value *ValueError
	if storage.asError() != nil || value.asError() != nil {
		t.Fatal("typed nil escaped")
	}
	storage = storageErr(errors.New("write"), "stdout")
	value = valueErr("flag")
	if storage == nil || value == nil {
		t.Fatal("nil constructor")
	}
	if !errors.Is(storage.asError(), storage) || !errors.Is(value.asError(), value) {
		t.Fatal("conversion lost identity")
	}
}
