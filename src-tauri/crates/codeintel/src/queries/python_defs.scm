(module
  (function_definition name: (identifier) @name) @def.fn)

(module
  (class_definition name: (identifier) @name) @def.class)

(module
  (decorated_definition
    definition: (function_definition name: (identifier) @name) @def.fn))

(module
  (decorated_definition
    definition: (class_definition name: (identifier) @name) @def.class))

(class_definition
  body: (block (function_definition name: (identifier) @name) @def.method))
