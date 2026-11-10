// Narrow adapter for the pinned Slang COM interfaces. Rust owns filesystem
// observations and immutable replay; no Slang repro container is involved.
#include <atomic>
#include <cstring>
#include <slang.h>
#include <stdexcept>
#include <string>
#include <vector>
#ifdef _WIN32
#include <windows.h>
#else
#include <dlfcn.h>
#endif

using Read = int (*)(void *, int, const char *, const char *,
                     const unsigned char **, size_t *);
using Emit = void (*)(void *, int, const char *, const void *, size_t);
static bool same(const SlangUUID &a, const SlangUUID &b) {
  return !memcmp(&a, &b, sizeof(a));
}
struct Blob final : ISlangBlob {
  std::atomic<unsigned> refs{1};
  std::vector<unsigned char> bytes;
  Blob(const void *p, size_t n) : bytes(n + 1, 0) {
    if (n)
      memcpy(bytes.data(), p, n);
  }
  SlangResult SLANG_MCALL queryInterface(const SlangUUID &id,
                                         void **out) override {
    *out = nullptr;
    if (!same(id, ISlangUnknown::getTypeGuid()) &&
        !same(id, ISlangBlob::getTypeGuid()))
      return SLANG_E_NO_INTERFACE;
    *out = this;
    addRef();
    return SLANG_OK;
  }
  uint32_t SLANG_MCALL addRef() override { return ++refs; }
  uint32_t SLANG_MCALL release() override {
    auto n = --refs;
    if (!n)
      delete this;
    return n;
  }
  const void *SLANG_MCALL getBufferPointer() override { return bytes.data(); }
  size_t SLANG_MCALL getBufferSize() override { return bytes.size() - 1; }
};
struct Files final : ISlangFileSystemExt {
  void *context;
  Read read;
  Files(void *c, Read r) : context(c), read(r) {}
  // Request-scoped: the owner outlives the request and every borrowed pointer.
  uint32_t SLANG_MCALL addRef() override { return 1; }
  uint32_t SLANG_MCALL release() override { return 1; }
  void *SLANG_MCALL castAs(const SlangUUID &id) override {
    return same(id, ISlangUnknown::getTypeGuid()) ||
                   same(id, ISlangCastable::getTypeGuid()) ||
                   same(id, ISlangFileSystem::getTypeGuid()) ||
                   same(id, ISlangFileSystemExt::getTypeGuid())
               ? this
               : nullptr;
  }
  SlangResult SLANG_MCALL queryInterface(const SlangUUID &id,
                                         void **out) override {
    *out = castAs(id);
    return *out ? SLANG_OK : SLANG_E_NO_INTERFACE;
  }
  SlangResult blob(int op, const char *path, const char *extra,
                   ISlangBlob **out) {
    *out = nullptr;
    const unsigned char *bytes = nullptr;
    size_t size = 0;
    if (read(context, op, path, extra, &bytes, &size) < 0)
      return SLANG_E_NOT_FOUND;
    *out = new Blob(bytes, size);
    return SLANG_OK;
  }
  SlangResult SLANG_MCALL loadFile(const char *p, ISlangBlob **out) override {
    return blob(0, p, nullptr, out);
  }
  SlangResult SLANG_MCALL getFileUniqueIdentity(const char *p,
                                                ISlangBlob **out) override {
    return blob(1, p, nullptr, out);
  }
  SlangResult SLANG_MCALL calcCombinedPath(SlangPathType type, const char *from,
                                           const char *p,
                                           ISlangBlob **out) override {
    return blob(type == SLANG_PATH_TYPE_FILE ? 4 : 5, from, p, out);
  }
  SlangResult SLANG_MCALL getPathType(const char *p,
                                      SlangPathType *out) override {
    const unsigned char *data = nullptr;
    size_t n = 0;
    if (read(context, 2, p, nullptr, &data, &n) < 0 || n != 1)
      return SLANG_E_NOT_FOUND;
    *out = data[0] ? SLANG_PATH_TYPE_FILE : SLANG_PATH_TYPE_DIRECTORY;
    return SLANG_OK;
  }
  SlangResult SLANG_MCALL getPath(PathKind kind, const char *p,
                                  ISlangBlob **out) override {
    if (kind == PathKind::OperatingSystem) {
      *out = nullptr;
      return SLANG_E_NOT_IMPLEMENTED;
    }
    return blob(kind == PathKind::Canonical ? 1 : 3, p, nullptr, out);
  }
  void SLANG_MCALL clearCache() override {}
  SlangResult SLANG_MCALL enumeratePathContents(const char *,
                                                FileSystemContentsCallBack,
                                                void *) override {
    return SLANG_E_NOT_IMPLEMENTED;
  }
  OSPathKind SLANG_MCALL getOSPathKind() override { return OSPathKind::None; }
};
struct Writer final : ISlangWriter {
  void *context;
  Emit emit;
  int channel;
  std::vector<char> buffer;
  Writer(void *c, Emit e, int ch) : context(c), emit(e), channel(ch) {}
  uint32_t SLANG_MCALL addRef() override { return 1; }
  uint32_t SLANG_MCALL release() override { return 1; }
  SlangResult SLANG_MCALL queryInterface(const SlangUUID &id,
                                         void **out) override {
    *out = nullptr;
    if (!same(id, ISlangUnknown::getTypeGuid()) &&
        !same(id, ISlangWriter::getTypeGuid()))
      return SLANG_E_NO_INTERFACE;
    *out = this;
    return SLANG_OK;
  }
  char *SLANG_MCALL beginAppendBuffer(size_t n) override {
    buffer.resize(n);
    return buffer.data();
  }
  SlangResult SLANG_MCALL endAppendBuffer(char *p, size_t n) override {
    return write(p, n);
  }
  SlangResult SLANG_MCALL write(const char *p, size_t n) override {
    emit(context, channel, nullptr, p, n);
    return SLANG_OK;
  }
  void SLANG_MCALL flush() override {}
  SlangBool SLANG_MCALL isConsole() override { return false; }
  SlangResult SLANG_MCALL setMode(SlangWriterMode) override { return SLANG_OK; }
};
struct Library {
#ifdef _WIN32
  HMODULE handle;
  Library(const char *path) : handle(nullptr) {
    int n = MultiByteToWideChar(CP_UTF8, MB_ERR_INVALID_CHARS, path, -1,
                                nullptr, 0);
    if (n) {
      std::vector<wchar_t> wide(n);
      MultiByteToWideChar(CP_UTF8, MB_ERR_INVALID_CHARS, path, -1, wide.data(),
                          n);
      handle = LoadLibraryExW(wide.data(), nullptr,
                              LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR |
                                  LOAD_LIBRARY_SEARCH_DEFAULT_DIRS);
    }
  }
  ~Library() {
    if (handle)
      FreeLibrary(handle);
  }
  void *symbol(const char *name) {
    return (void *)GetProcAddress(handle, name);
  }
#else
  void *handle;
  Library(const char *path) : handle(dlopen(path, RTLD_NOW | RTLD_LOCAL)) {}
  ~Library() {
    if (handle)
      dlclose(handle);
  }
  void *symbol(const char *name) { return dlsym(handle, name); }
#endif
  template <typename T> T get(const char *name) {
    auto p = symbol(name);
    if (!p)
      throw std::runtime_error(std::string("missing Slang API: ") + name);
    return reinterpret_cast<T>(p);
  }
};
extern "C" int vork_slang_compile(const char *library,
                                  const char *compilerDirectory, int argc,
                                  const char *const *argv, void *context,
                                  Read read, Emit emit) noexcept {
  try {
    Library lib(library);
    if (!lib.handle)
      throw std::runtime_error(std::string("cannot load Slang library: ") +
                               library);
    auto create = lib.get<decltype(&spCreateSession)>("spCreateSession");
    auto destroy = lib.get<decltype(&spDestroySession)>("spDestroySession");
    auto requestCreate =
        lib.get<decltype(&spCreateCompileRequest)>("spCreateCompileRequest");
    auto requestDestroy =
        lib.get<decltype(&spDestroyCompileRequest)>("spDestroyCompileRequest");
    struct Session {
      SlangSession *p;
      decltype(destroy) drop;
      ~Session() {
        if (p)
          drop(p);
      }
    } session{create(nullptr), destroy};
    if (!session.p)
      throw std::runtime_error("cannot create Slang session");
    // Slang's optional SPIR-V backends belong to the selected toolchain, not
    // this companion's executable directory (notably for Windows DLL loading).
    std::string libraryPath(library);
    auto libraryDirectory = libraryPath.substr(0, libraryPath.find_last_of("/\\"));
    for (auto backend : {SLANG_PASS_THROUGH_GLSLANG, SLANG_PASS_THROUGH_SPIRV_OPT,
                         SLANG_PASS_THROUGH_SPIRV_DIS, SLANG_PASS_THROUGH_SPIRV_LINK})
      session.p->setDownstreamCompilerPath(backend, libraryDirectory.c_str());
    Files files(context, read);
    Writer stdoutWriter(context, emit, 1), stderrWriter(context, emit, 2);
    struct Request {
      SlangCompileRequest *p;
      decltype(requestDestroy) drop;
      ~Request() {
        if (p)
          drop(p);
      }
    } request{requestCreate(session.p), requestDestroy};
    auto r = request.p;
    if (!r)
      throw std::runtime_error("cannot create Slang request");
    r->setFileSystem(&files);
    r->setCommandLineCompilerMode();
    r->addSearchPath(compilerDirectory);
    r->setWriter(SLANG_WRITER_CHANNEL_STD_OUTPUT, &stdoutWriter);
    r->setWriter(SLANG_WRITER_CHANNEL_STD_ERROR, &stderrWriter);
    auto result = r->processCommandLineArguments(argv, argc);
    if (SLANG_SUCCEEDED(result))
      result = r->compile();
    const char *diagnostics = r->getDiagnosticOutput();
    if (diagnostics)
      emit(context, 2, nullptr, diagnostics, strlen(diagnostics));
    if (SLANG_FAILED(result))
      return result;
    // Let the same command-line output writer as slangc associate targets,
    // entry points and reflection outputs. All destinations belong to the
    // caller's workspace; only input resolution uses our filesystem.
    for (int i = 0; i + 1 < argc; i++) {
      if (!strcmp(argv[i], "-depfile")) {
        // Dependency paths are delivered separately; Rust serializes the
        // existing depfile format without interpreting Slang source.
        for (int d = 0; d < r->getDependencyFileCount(); d++) {
          auto p = r->getDependencyFilePath(d);
          emit(context, 3, argv[i + 1], p, strlen(p));
        }
        ++i;
      }
    }
    return 0;
  } catch (const std::exception &e) {
    emit(context, 2, nullptr, e.what(), strlen(e.what()));
    return -1;
  } catch (...) {
    const char *error = "unexpected exception in Slang filesystem adapter";
    emit(context, 2, nullptr, error, strlen(error));
    return -1;
  }
}

// The adapter runs in the existing guarded compiler process group. Besides
// crash isolation this gives Slang's AST/debug outputs a request-local cwd.
#include <cstdio>
#ifndef _WIN32
#include <unistd.h>
#endif
#include <cstdint>
#ifdef _WIN32
#include <fcntl.h>
#include <io.h>
#endif
static FILE *protocolOut;
static void put64(uint64_t n) {
  for (int i = 0; i < 8; i++)
    std::fputc((n >> (8 * i)) & 255, protocolOut);
}
static uint64_t get64() {
  uint64_t n = 0;
  for (int i = 0; i < 8; i++) {
    int c = std::getchar();
    if (c == EOF)
      std::exit(2);
    n |= uint64_t(c) << (8 * i);
  }
  return n;
}
static void put(const void *p, size_t n) {
  put64(n);
  if (n && std::fwrite(p, 1, n, protocolOut) != n)
    std::exit(2);
}
static void string(const char *p) { put(p, p ? std::strlen(p) : 0); }
static int remoteRead(void *context, int op, const char *p, const char *extra,
                      const unsigned char **data, size_t *size) {
  std::fputc('R', protocolOut);
  put64(op);
  string(p);
  string(extra);
  std::fflush(protocolOut);
  auto n = get64();
  if (n == UINT64_MAX)
    return -1;
  auto &bytes = *(std::vector<unsigned char> *)context;
  bytes.resize(n);
  if (n && std::fread(bytes.data(), 1, n, stdin) != n)
    std::exit(2);
  *data = bytes.data();
  *size = bytes.size();
  return 0;
}
static void remoteEmit(void *, int channel, const char *name, const void *data,
                       size_t size) {
  std::fputc('E', protocolOut);
  put64(channel);
  string(name);
  put(data, size);
  std::fflush(protocolOut);
}
int run(int argc, const char *const *argv) {
#ifdef _WIN32
  _setmode(_fileno(stdin), _O_BINARY);
  _setmode(_fileno(stdout), _O_BINARY);
#endif
  if (argc < 3)
    return 2;
  // Reserve the original stdout pipe for framing, then redirect incidental
  // compiler/downstream printf output to stderr. Registered Slang writers
  // still emit through the protocol, including -E and diagnostics.
#ifdef _WIN32
  protocolOut = _fdopen(_dup(_fileno(stdout)), "wb");
  _dup2(_fileno(stderr), _fileno(stdout));
#else
  protocolOut = fdopen(dup(fileno(stdout)), "wb");
  dup2(fileno(stderr), fileno(stdout));
#endif
  if (!protocolOut)
    return 2;
  put64(1);
  std::fflush(protocolOut);
  std::vector<unsigned char> bytes;
  auto result = vork_slang_compile(argv[1], argv[2], argc - 3, argv + 3, &bytes,
                                   remoteRead, remoteEmit);
  std::fputc('D', protocolOut);
  put64(uint32_t(result));
  std::fflush(protocolOut);
  return 0;
}

#ifdef _WIN32
int wmain(int argc, wchar_t **argv) {
  std::vector<std::string> arguments;
  for (int i = 0; i < argc; i++) {
    int n = WideCharToMultiByte(CP_UTF8, WC_ERR_INVALID_CHARS, argv[i], -1,
                                nullptr, 0, nullptr, nullptr);
    if (!n)
      return 2;
    std::string text(n, 0);
    WideCharToMultiByte(CP_UTF8, WC_ERR_INVALID_CHARS, argv[i], -1, text.data(),
                        n, nullptr, nullptr);
    text.pop_back();
    arguments.push_back(std::move(text));
  }
  std::vector<const char *> pointers;
  for (auto &argument : arguments)
    pointers.push_back(argument.c_str());
  return run(argc, pointers.data());
}
#else
int main(int argc, char **argv) { return run(argc, argv); }
#endif
