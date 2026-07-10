; Call: foo(...)
(call_expression function: (identifier) @name) @ref.call

; Method call: x.foo(...)
(call_expression function: (field_expression field: (field_identifier) @name)) @ref.call

; Type reference: Foo
(type_identifier) @name @ref.reference
