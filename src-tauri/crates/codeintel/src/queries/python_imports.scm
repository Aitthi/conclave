; from .user import User, new_user as nu
(import_from_statement
  module_name: (_) @path
  name: (dotted_name (identifier) @name)) @import

(import_from_statement
  module_name: (_) @path
  name: (aliased_import
    name: (dotted_name (identifier) @name)
    alias: (identifier) @alias)) @import

; import foo
(import_statement
  name: (dotted_name (identifier) @name)) @import

; import foo as bar
(import_statement
  name: (aliased_import
    name: (dotted_name (identifier) @name)
    alias: (identifier) @alias)) @import

; from foo import Bar as Baz    -- alias "re-export" (MVP: treat all aliased
; imports as candidate re-export sites; agent verifies via __all__ manually)
(import_from_statement
  module_name: (dotted_name) @path
  name: (aliased_import
    name: (dotted_name) @original
    alias: (identifier) @alias)) @reexport_alias

; from foo import *             -- wildcard "re-export"
(import_from_statement
  module_name: (dotted_name) @path
  (wildcard_import)) @reexport_wildcard
