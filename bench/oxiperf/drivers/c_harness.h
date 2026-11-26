/* OxiPerf C driver. Included at the end of each suites/<w>/main.c, which defines
   uint32_t run(int32_t n). Usage: prog <n> <warmup> <iters> <min_iter_ms>
   Prints one JSON line (the child protocol shared by every executor). */
#include <stdio.h>
#include <stdint.h>
#include <stdlib.h>
#include <time.h>
#include <sys/resource.h>

/* Opaque to the optimiser: without it gcc may hoist the pure `run(n)` out of the timing loop. */
static int32_t oxiperf_opaque(int32_t n) { __asm__ volatile("" : "+r"(n)); return n; }

static uint64_t oxiperf_now_ns(void) {
  struct timespec ts;
  clock_gettime(CLOCK_MONOTONIC, &ts);
  return (uint64_t)ts.tv_sec * 1000000000ull + (uint64_t)ts.tv_nsec;
}

int main(int argc, char **argv) {
  if (argc < 5) { fprintf(stderr, "usage: %s n warmup iters min_iter_ms\n", argv[0]); return 2; }
  int32_t n = atoi(argv[1]);
  int warmup = atoi(argv[2]), iters = atoi(argv[3]);
  double min_ms = atof(argv[4]);
  uint64_t t0 = oxiperf_now_ns();
  volatile uint32_t sum = run(oxiperf_opaque(n));
  uint64_t cold = oxiperf_now_ns() - t0;
  uint32_t expect = sum, stable = 1;
  uint64_t last = cold;
  for (int i = 0; i < warmup; i++) {
    uint64_t a = oxiperf_now_ns();
    sum = run(oxiperf_opaque(n));
    last = oxiperf_now_ns() - a;
    if (sum != expect) stable = 0;
  }
  int reps = (int)((min_ms * 1e6) / (double)(last ? last : 1)) + 1;
  if (reps < 1) reps = 1;
  double *ns = malloc(sizeof(double) * (iters > 0 ? iters : 1));
  for (int i = 0; i < iters; i++) {
    uint64_t a = oxiperf_now_ns();
    for (int r = 0; r < reps; r++) { sum = run(oxiperf_opaque(n)); if (sum != expect) stable = 0; }
    ns[i] = (double)(oxiperf_now_ns() - a) / reps;
  }
  struct rusage ru;
  getrusage(RUSAGE_SELF, &ru);
  printf("{\"checksum\":%u,\"checksum_stable\":%s,\"cold_ns\":%llu,\"warmup\":%d,\"inner_reps\":%d,\"peak_rss_kib\":%ld,\"iter_ns\":[",
         expect, stable ? "true" : "false", (unsigned long long)cold, warmup, reps, ru.ru_maxrss);
  for (int i = 0; i < iters; i++) printf("%s%.1f", i ? "," : "", ns[i]);
  printf("]}\n");
  return 0;
}
