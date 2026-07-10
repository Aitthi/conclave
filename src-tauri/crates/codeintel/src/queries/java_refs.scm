; Method call: add(1, 2) / obj.add(1, 2)
(method_invocation name: (identifier) @name) @ref.call

; Constructor call: new Foo(...)
(object_creation_expression type: (type_identifier) @name) @ref.call

; Type reference: Foo
(type_identifier) @name @ref.reference
