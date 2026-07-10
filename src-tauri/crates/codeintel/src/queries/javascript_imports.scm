(import_statement
  (import_clause
    (named_imports
      (import_specifier
        name: (identifier) @name
        alias: (identifier)? @alias)))
  source: (string) @path) @import

(import_statement
  (import_clause (identifier) @name)
  source: (string) @path) @import

(import_statement
  (import_clause (namespace_import (identifier) @name))
  source: (string) @path) @import

; export { Bar as Baz } from "./foo";
(export_statement
  (export_clause
    (export_specifier
      name: (identifier) @original
      alias: (identifier) @alias))
  source: (string) @path) @reexport_alias

; export * from "./foo";
(export_statement
  "*"
  source: (string) @path) @reexport_wildcard
