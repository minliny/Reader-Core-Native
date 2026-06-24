cmake_minimum_required(VERSION 3.18.1)
project(reader_core_jni CXX)

set(CMAKE_CXX_STANDARD 17)
set(CMAKE_CXX_STANDARD_REQUIRED ON)
set(CMAKE_POSITION_INDEPENDENT_CODE ON)

# Reader-Core static library + headers are produced outside CMake (cargo).
# Drive this file with:
#   -DREADER_CORE_STATICLIB=<abs path to libreader_core.a>
#   -DREADER_CORE_INCLUDE_DIR=<abs path to include/ (contains reader_core.h)>
if(NOT DEFINED READER_CORE_STATICLIB OR NOT EXISTS "${READER_CORE_STATICLIB}")
  message(FATAL_ERROR
    "READER_CORE_STATICLIB must point to the built libreader_core.a")
endif()
if(NOT DEFINED READER_CORE_INCLUDE_DIR OR NOT EXISTS "${READER_CORE_INCLUDE_DIR}/reader_core.h")
  message(FATAL_ERROR
    "READER_CORE_INCLUDE_DIR must point to the directory containing reader_core.h")
endif()

add_library(reader_core_jni SHARED
  ${CMAKE_CURRENT_SOURCE_DIR}/jni/reader_jni.cpp)

target_include_directories(reader_core_jni PRIVATE
  ${READER_CORE_INCLUDE_DIR})

# Whole-archive so every FFI symbol from the Rust static lib is pulled into
# the shared library — JNI resolves them at load time and dead-stripping
# would otherwise drop them.
target_link_options(reader_core_jni PRIVATE
  "LINKER:--whole-archive"
  "${READER_CORE_STATICLIB}"
  "LINKER:--no-whole-archive")

if(ANDROID)
  target_link_libraries(reader_core_jni PRIVATE log dl m)
endif()
