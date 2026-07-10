(function_item name: (identifier) @name) @def.fn
(struct_item   name: (type_identifier) @name) @def.struct
(enum_item     name: (type_identifier) @name) @def.enum
(trait_item    name: (type_identifier) @name) @def.trait
(type_item     name: (type_identifier) @name) @def.type
(const_item    name: (identifier) @name)      @def.const
(static_item   name: (identifier) @name)      @def.const

; Methods sit inside `impl` blocks. We capture them separately so they get DefKind::Method.
(impl_item
  body: (declaration_list
    (function_item name: (identifier) @name) @def.method))
