// Clang lowers `@available(macOS X, *)` / `__builtin_available` to a call to the
// compiler-runtime helper `__isPlatformVersionAtLeast`. Apple's clang links that
// automatically from libclang_rt.osx.a; osxcross ships no such darwin runtime, so
// libslic3r's MacUtils.mm (which uses @available) leaves the symbol undefined at
// the shim's final link. Provide it via the public Foundation API.
//
// IMPORTANT: compiled ONLY for the Linux→macOS osxcross cross build
// (N3O_MACOS_CROSS, set by build.rs). A native macOS build — including the
// Apple-Silicon→Intel cross — gets the real symbol from the toolchain, and
// defining our own there would be a duplicate symbol.

#import <Foundation/Foundation.h>
#include <stdint.h>

extern "C" int32_t __isPlatformVersionAtLeast(uint32_t /*Platform*/, uint32_t Major,
                                              uint32_t Minor, uint32_t Subminor) {
    // Our only macOS target is macOS itself, so the platform arg is ignored: a
    // plain OS-version comparison is what every @available(macOS …) check needs.
    NSOperatingSystemVersion req = {(NSInteger)Major, (NSInteger)Minor, (NSInteger)Subminor};
    return [[NSProcessInfo processInfo] isOperatingSystemAtLeastVersion:req] ? 1 : 0;
}
