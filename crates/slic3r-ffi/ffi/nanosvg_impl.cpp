// nanosvg_impl.cpp — instantiate the header-only nanosvg library.
//
// libslic3r references nsvg* / nsvgRast* functions (Format/svg.cpp,
// NSVGUtils.cpp) but never compiles the implementation; upstream OrcaSlicer
// instantiates it in src/slic3r/GUI/BitmapCache.cpp, which is only built when
// SLIC3R_GUI=ON. The FFI shim builds with SLIC3R_GUI=OFF so we must supply
// our own instantiation here.

#define NANOSVG_IMPLEMENTATION
#include "nanosvg/nanosvg.h"

#define NANOSVGRAST_IMPLEMENTATION
#include "nanosvg/nanosvgrast.h"
