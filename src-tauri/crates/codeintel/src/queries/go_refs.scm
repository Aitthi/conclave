; Simple call: foo(...)
(call_expression function: (identifier) @name) @ref.call

; Selector call: pkg.Foo(...) / x.method(...)
(call_expression function: (selector_expression field: (field_identifier) @name)) @ref.call

; Type reference: Foo
(type_identifier) @name @ref.reference
