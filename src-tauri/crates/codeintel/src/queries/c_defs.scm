(function_definition declarator: (function_declarator declarator: (identifier) @name)) @def.fn
(struct_specifier name: (type_identifier) @name body: (field_declaration_list)) @def.struct
(enum_specifier name: (type_identifier) @name body: (enumerator_list)) @def.enum
(type_definition declarator: (type_identifier) @name) @def.type
