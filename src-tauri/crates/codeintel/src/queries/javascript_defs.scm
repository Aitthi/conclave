(function_declaration name: (identifier) @name) @def.fn
(class_declaration name: (identifier) @name) @def.class
(method_definition name: (property_identifier) @name) @def.method
(lexical_declaration "const" (variable_declarator name: (identifier) @name)) @def.const
