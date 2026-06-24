cmake_minimum_required(VERSION 3.16)

project(reader_core_harmony_napi LANGUAGES C CXX)

set(CMAKE_CXX_STANDARD 17)
set(CMAKE_CXX_STANDARD_REQUIRED ON)
set(CMAKE_CXX_EXTENSIONS OFF)

if(NOT DEFINED READER_CORE_NATIVE_ROOT)
  get_filename_component(READER_CORE_NATIVE_ROOT
    "${CMAKE_CURRENT_LIST_DIR}/../../.." ABSOLUTE)
endif()

set(READER_CORE_TARGET_TRIPLE "aarch64-unknown-linux-ohos"
  CACHE STRING "Rust target triple used for the Reader Core static library")
set(READER_CORE_STATICLIB
  "${READER_CORE_NATIVE_ROOT}/target/${READER_CORE_TARGET_TRIPLE}/release/libreader_core.a")

if(NOT EXISTS "${READER_CORE_STATICLIB}")
  message(FATAL_ERROR
    "Missing ${READER_CORE_STATICLIB}. Run scripts/build-ohos.sh first.")
endif()

add_library(reader_core_static STATIC IMPORTED GLOBAL)
set_target_properties(reader_core_static PROPERTIES
  IMPORTED_LOCATION "${READER_CORE_STATICLIB}")

add_library(reader_core_napi SHARED
  reader_napi.cpp)

target_include_directories(reader_core_napi PRIVATE
  "${READER_CORE_NATIVE_ROOT}/include")

find_library(ACE_NAPI_LIB ace_napi.z REQUIRED)

target_link_libraries(reader_core_napi PRIVATE
  reader_core_static
  ${ACE_NAPI_LIB})

set_target_properties(reader_core_napi PROPERTIES
  OUTPUT_NAME "reader_core_napi")
