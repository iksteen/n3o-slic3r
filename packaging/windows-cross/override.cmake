# CMAKE_USER_MAKE_RULES_OVERRIDE — clang-cl quirks (from cargo-xwin's generated
# override). Included after the default compile rules, before the project.
#
# clang-cl reads paths starting with /U as macro-undefines, so source paths
# need a `--` separator. And llvm-rc wants -D not /D for defines.
string(REPLACE "-c <SOURCE>" "-c -- <SOURCE>" CMAKE_C_COMPILE_OBJECT "${CMAKE_C_COMPILE_OBJECT}")
string(REPLACE "-c <SOURCE>" "-c -- <SOURCE>" CMAKE_CXX_COMPILE_OBJECT "${CMAKE_CXX_COMPILE_OBJECT}")
string(REPLACE "/D" "-D" CMAKE_RC_FLAGS "${CMAKE_RC_FLAGS_INIT}")
string(REPLACE "/D" "-D" CMAKE_RC_FLAGS_DEBUG "${CMAKE_RC_FLAGS_DEBUG_INIT}")
if(NOT CMAKE_HOST_WIN32)
  set(CMAKE_NINJA_CMCLDEPS_RC 0) # cmcldeps is Windows-only
endif()
