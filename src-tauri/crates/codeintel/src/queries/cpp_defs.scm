(function_definition declarator: (function_declarator declarator: (identifier) @name)) @def.fn
(struct_specifier name: (type_identifier) @name body: (field_declaration_list)) @def.struct
(enum_specifier name: (type_identifier) @name body: (enumerator_list)) @def.enum
(type_definition declarator: (type_identifier) @name) @def.type
(class_specifier name: (type_identifier) @name body: (field_declaration_list)) @def.class

; Out-of-class method definition: int Shape::area() { ... }
(function_definition declarator: (function_declarator declarator: (qualified_identifier name: (identifier) @name))) @def.method

; In-class method definition
(function_definition declarator: (function_declarator declarator: (field_identifier) @name)) @def.method
