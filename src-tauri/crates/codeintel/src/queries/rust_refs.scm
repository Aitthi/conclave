; Simple call: foo(...)
(call_expression
  function: (identifier) @name) @ref.call

; Path call: foo::bar(...)
(call_expression
  function: (scoped_identifier
    name: (identifier) @name)) @ref.call

; Method call: x.foo(...)
(call_expression
  function: (field_expression
    field: (field_identifier) @name)) @ref.call

; Macro invocation: println!(...)
(macro_invocation
  macro: (identifier) @name) @ref.call

; Type reference (handles fn signatures, struct fields, generics): Foo
(type_identifier) @name @ref.reference

; Path expression (e.g. `Foo::CONST` reads `Foo`): used as a non-call identifier reference.
(scoped_identifier
  name: (identifier) @name) @ref.reference
