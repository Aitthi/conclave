; #include "local.h"
(preproc_include path: (string_literal) @path) @import

; #include <stdio.h>
(preproc_include path: (system_lib_string) @path) @import
