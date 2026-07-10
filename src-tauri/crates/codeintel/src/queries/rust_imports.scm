; Single path: use crate::auth;     use std::collections::HashMap;
(use_declaration
  argument: (scoped_identifier
    path: (_) @path
    name: (identifier) @name)) @import

; Aliased single path: use crate::auth as a;
(use_declaration
  argument: (use_as_clause
    path: (_) @path
    alias: (identifier) @alias)) @import

; Grouped: use crate::{a, b as bb};
(use_declaration
  argument: (scoped_use_list
    path: (_) @path
    list: (use_list) @group)) @import

; Glob: use crate::auth::*;
; The grammar represents `path::*` as use_wildcard { child: path_node },
; NOT as scoped_use_list { list: use_wildcard } — the latter is impossible
; per node-types.json (scoped_use_list.list only accepts use_list).
(use_declaration
  argument: (use_wildcard
    (_) @path)) @import

; pub use foo::Bar as Baz;   -- alias re-export
(use_declaration
  (visibility_modifier) @vis
  argument: (use_as_clause
    path: (_) @path
    alias: (identifier) @alias)) @reexport_alias

; pub use foo::*;            -- wildcard re-export
(use_declaration
  (visibility_modifier) @vis
  argument: (use_wildcard
    (_) @path)) @reexport_wildcard
