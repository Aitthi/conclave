; Plain import: import "fmt"
(import_spec path: (interpreted_string_literal) @path) @import

; Aliased import: import f "fmt"
(import_spec name: (package_identifier) @alias path: (interpreted_string_literal) @path) @import
