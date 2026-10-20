// doccheck enforces the documentation rule of this repository: every top-level constant, variable and type,
// and every struct field, must have a comment. It prints the offending declarations and exits with status 1
// if there are any. Test files are skipped.
//
//	go run ./tools/doccheck agentbus/*.go live/*.go    (from tools/doccheck: go build, then run the binary)
package main

import (
	"fmt"
	"go/ast"
	"go/parser"
	"go/token"
	"os"
	"strings"
)

// main checks the files given as arguments.
func main() {
	fs := token.NewFileSet()
	missing := 0
	report := func(pos token.Pos, what string) {
		fmt.Printf("%s: undocumented %s\n", fs.Position(pos), what)
		missing++
	}
	for _, path := range os.Args[1:] {
		if strings.HasSuffix(path, "_test.go") {
			continue
		}
		f, err := parser.ParseFile(fs, path, nil, parser.ParseComments)
		if err != nil {
			fmt.Fprintln(os.Stderr, err)
			os.Exit(2)
		}
		for _, d := range f.Decls {
			g, ok := d.(*ast.GenDecl)
			if !ok || g.Tok == token.IMPORT {
				continue
			}
			for _, sp := range g.Specs {
				switch s := sp.(type) {
				case *ast.ValueSpec:
					// a group comment is enough only for single-item groups
					if s.Doc == nil && s.Comment == nil && (g.Doc == nil || len(g.Specs) > 1) {
						report(s.Pos(), g.Tok.String()+" "+s.Names[0].Name)
					}
				case *ast.TypeSpec:
					if s.Doc == nil && g.Doc == nil {
						report(s.Pos(), "type "+s.Name.Name)
					}
					if st, ok := s.Type.(*ast.StructType); ok {
						for _, fld := range st.Fields.List {
							if fld.Doc == nil && fld.Comment == nil {
								name := "(embedded)"
								if len(fld.Names) > 0 {
									name = fld.Names[0].Name
								}
								report(fld.Pos(), "field "+s.Name.Name+"."+name)
							}
						}
					}
				}
			}
		}
	}
	if missing > 0 {
		fmt.Printf("%d undocumented declarations\n", missing)
		os.Exit(1)
	}
}
