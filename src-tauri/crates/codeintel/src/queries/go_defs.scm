(function_declaration name: (identifier) @name) @def.fn
(method_declaration name: (field_identifier) @name) @def.method
(type_declaration (type_spec name: (type_identifier) @name type: (struct_type))) @def.struct
(type_declaration (type_spec name: (type_identifier) @name type: (interface_type))) @def.interface
(type_declaration (type_spec name: (type_identifier) @name)) @def.type
(const_declaration (const_spec name: (identifier) @name)) @def.const
