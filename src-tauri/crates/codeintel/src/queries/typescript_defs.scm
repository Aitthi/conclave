(function_declaration name: (identifier) @name) @def.fn
(class_declaration name: (type_identifier) @name) @def.class
(interface_declaration name: (type_identifier) @name) @def.interface
(type_alias_declaration name: (type_identifier) @name) @def.type
(enum_declaration name: (identifier) @name) @def.enum
(lexical_declaration "const" (variable_declarator name: (identifier) @name)) @def.const

; Methods inside classes.
(class_body (method_definition name: (property_identifier) @name) @def.method)
