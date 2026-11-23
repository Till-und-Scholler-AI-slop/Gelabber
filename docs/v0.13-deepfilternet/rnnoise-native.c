/* GEL-28: the pinned RNNoise built natively, as the desktop app links it
 * (desktop/native/CMakeLists.txt), timed per 10 ms frame.
 * Usage: rnnoise-native <out-dir> <in.wav>...   (48 kHz mono float WAV as
 * written by mix.mjs). Prints one JSON object; writes <out-dir>/<name>. */
#include <math.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/resource.h>
#include <time.h>

#include "rnnoise.h"

#define FRAME 480

static float *read_f32_wav(const char *path, size_t *frames) {
  FILE *f = fopen(path, "rb");
  if (!f) return NULL;
  unsigned char h[44];
  if (fread(h, 1, 44, f) != 44 || memcmp(h, "RIFF", 4) || memcmp(h + 8, "WAVE", 4) ||
      h[20] != 3 || h[22] != 1 || h[34] != 32) {
    fclose(f);
    return NULL;
  }
  uint32_t bytes = h[40] | h[41] << 8 | h[42] << 16 | (uint32_t)h[43] << 24;
  *frames = bytes / 4;
  float *x = malloc(bytes);
  if (fread(x, 4, *frames, f) != *frames) {
    free(x);
    x = NULL;
  }
  fclose(f);
  return x;
}

static void write_f32_wav(const char *path, const float *x, size_t frames) {
  FILE *f = fopen(path, "wb");
  uint32_t data = frames * 4, riff = 36 + data, rate = 48000, byte_rate = 48000 * 4, fmt = 16;
  uint16_t format = 3, ch = 1, align = 4, bits = 32;
  fwrite("RIFF", 1, 4, f);
  fwrite(&riff, 4, 1, f);
  fwrite("WAVEfmt ", 1, 8, f);
  fwrite(&fmt, 4, 1, f);
  fwrite(&format, 2, 1, f);
  fwrite(&ch, 2, 1, f);
  fwrite(&rate, 4, 1, f);
  fwrite(&byte_rate, 4, 1, f);
  fwrite(&align, 2, 1, f);
  fwrite(&bits, 2, 1, f);
  fwrite("data", 1, 4, f);
  fwrite(&data, 4, 1, f);
  fwrite(x, 4, frames, f);
  fclose(f);
}

static double clock_ms(clockid_t id) {
  struct timespec t;
  clock_gettime(id, &t);
  return t.tv_sec * 1e3 + t.tv_nsec / 1e6;
}

static int cmp(const void *a, const void *b) {
  double x = *(const double *)a, y = *(const double *)b;
  return (x > y) - (x < y);
}

static void print_stats(double *t, size_t n) {
  qsort(t, n, sizeof *t, cmp);
  double sum = 0;
  for (size_t i = 0; i < n; i++) sum += t[i];
  size_t p99 = (size_t)(0.99 * n);
  printf("{\"frames\":%zu,\"medianMs\":%.6f,\"p99Ms\":%.6f,\"maxMs\":%.6f,\"meanMs\":%.6f}", n,
         t[n / 2], t[p99 < n ? p99 : n - 1], t[n - 1], sum / n);
}

int main(int argc, char **argv) {
  if (argc < 3) {
    fprintf(stderr, "usage: rnnoise-native <out-dir> <in.wav>...\n");
    return 2;
  }
  size_t cap = 1 << 20, count = 0;
  /* Wall time and this thread's CPU time per frame: on a loaded machine
   * only the second is unaffected by other processes taking the core. */
  double *all = malloc(cap * sizeof *all), *all_cpu = malloc(cap * sizeof *all_cpu);
  struct rusage r0, r1;
  getrusage(RUSAGE_SELF, &r0);
  for (int a = 2; a < argc; a++) {
    size_t frames;
    float *x = read_f32_wav(argv[a], &frames);
    if (!x) {
      fprintf(stderr, "%s: need a float32 mono WAV\n", argv[a]);
      return 1;
    }
    float *y = calloc(frames, sizeof *y), in[FRAME], out[FRAME];
    DenoiseState *st = rnnoise_create(NULL);
    for (size_t n = 0; n + FRAME <= frames; n += FRAME) {
      for (int j = 0; j < FRAME; j++) in[j] = x[n + j] * 32768.f;
      double c = clock_ms(CLOCK_THREAD_CPUTIME_ID), t = clock_ms(CLOCK_MONOTONIC);
      rnnoise_process_frame(st, out, in);
      double wall = clock_ms(CLOCK_MONOTONIC) - t, cpu = clock_ms(CLOCK_THREAD_CPUTIME_ID) - c;
      if (count == cap) {
        cap *= 2;
        all = realloc(all, cap * sizeof *all);
        all_cpu = realloc(all_cpu, cap * sizeof *all_cpu);
      }
      all_cpu[count] = cpu;
      all[count++] = wall;
      for (int j = 0; j < FRAME; j++) y[n + j] = out[j] / 32768.f;
    }
    rnnoise_destroy(st);
    const char *name = strrchr(argv[a], '/');
    char path[4096];
    snprintf(path, sizeof path, "%s/%s", argv[1], name ? name + 1 : argv[a]);
    write_f32_wav(path, y, frames);
    free(x);
    free(y);
  }
  getrusage(RUSAGE_SELF, &r1);
  double cpu = (r1.ru_utime.tv_sec - r0.ru_utime.tv_sec) + (r1.ru_utime.tv_usec - r0.ru_utime.tv_usec) / 1e6 +
               (r1.ru_stime.tv_sec - r0.ru_stime.tv_sec) + (r1.ru_stime.tv_usec - r0.ru_stime.tv_usec) / 1e6;
  printf("{\"engine\":\"RNNoise v0.2 (pinned source, native %s)\",\"frameSamples\":%d,\"compute\":",
#ifdef __clang__
         "clang " __clang_version__,
#else
         "gcc " __VERSION__,
#endif
         FRAME);
  print_stats(all, count);
  printf(",\"threadCpu\":");
  print_stats(all_cpu, count);
  printf(",\"processCpuSeconds\":%.2f}\n", cpu);
  return 0;
}
