/* Bruto line-profiler runtime. Linked only into profile builds.
 *
 * Codegen calls __bruto_prof_enter(id) at routine entry, __bruto_prof_exit()
 * before each return, and __bruto_prof_line(id) at each statement. The
 * runtime keeps a call tree keyed by (parent node, location id) in a fixed
 * open-addressing hash table and writes it to $BRUTO_PROF_OUT at exit.
 *
 * File format: see docs/superpowers/specs/2026-09-17-line-profiler-design.md
 * ("Profile file").
 */
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#if defined(_WIN32)
#include <windows.h>
static uint64_t bp_now(void) {
    static LARGE_INTEGER freq;
    LARGE_INTEGER t;
    if (freq.QuadPart == 0) QueryPerformanceFrequency(&freq);
    QueryPerformanceCounter(&t);
    return (uint64_t)((double)t.QuadPart * 1e9 / (double)freq.QuadPart);
}
#else
#include <time.h>
static uint64_t bp_now(void) {
    struct timespec ts;
    clock_gettime(CLOCK_MONOTONIC, &ts);
    return (uint64_t)ts.tv_sec * 1000000000ull + (uint64_t)ts.tv_nsec;
}
#endif

#define BP_CAPACITY 65536u   /* nodes; power of two */
#define BP_STACK    4096u
#define BP_NONE     0xFFFFFFFFu
#define BP_KIND_ROUTINE 1
#define BP_KIND_LINE    2
#define BP_FLAG_TABLE_FULL     1u
#define BP_FLAG_STACK_OVERFLOW 2u

typedef struct {
    uint8_t  used;
    uint8_t  kind;
    uint32_t loc;
    uint32_t parent;     /* node index or BP_NONE */
    uint64_t calls;
    uint64_t self_ns;
    uint64_t total_ns;
} bp_node;

typedef struct {
    uint32_t node;       /* routine node for this frame */
    uint64_t entered;    /* bp_now() at enter */
    uint64_t child_ns;   /* total time of callees, for self time */
    uint32_t cur_line;   /* line node currently running or BP_NONE */
    uint64_t line_start; /* bp_now() when cur_line began */
    uint64_t line_child_ns; /* callee time inside cur_line */
} bp_frame;

static bp_node  bp_nodes[BP_CAPACITY];
static uint32_t bp_count = 0;
static bp_frame bp_stack[BP_STACK];
static uint32_t bp_depth = 0;      /* frames in use */
static uint32_t bp_overflow = 0;   /* enters beyond BP_STACK, ignored */
static uint32_t bp_flags = 0;
static uint64_t bp_start = 0;
static int bp_registered = 0;

static void bp_write(void);

static uint32_t bp_find(uint32_t parent, uint32_t loc, uint8_t kind) {
    uint32_t h = (parent * 2654435761u) ^ (loc * 40503u);
    for (uint32_t probe = 0; probe < BP_CAPACITY; probe++) {
        uint32_t i = (h + probe) & (BP_CAPACITY - 1);
        bp_node *n = &bp_nodes[i];
        if (!n->used) {
            if (bp_count + 1 >= BP_CAPACITY) { bp_flags |= BP_FLAG_TABLE_FULL; return BP_NONE; }
            n->used = 1; n->kind = kind; n->loc = loc; n->parent = parent;
            bp_count++;
            return i;
        }
        if (n->loc == loc && n->parent == parent && n->kind == kind) return i;
    }
    bp_flags |= BP_FLAG_TABLE_FULL;
    return BP_NONE;
}

/* Close the running line of the top frame, charging it self time. */
static void bp_close_line(bp_frame *f, uint64_t now) {
    if (f->cur_line == BP_NONE) return;
    uint64_t span = now - f->line_start;
    bp_node *ln = &bp_nodes[f->cur_line];
    ln->total_ns += span;
    ln->self_ns += span > f->line_child_ns ? span - f->line_child_ns : 0;
    f->cur_line = BP_NONE;
    f->line_child_ns = 0;
}

void __bruto_prof_enter(uint32_t loc) {
    uint64_t now = bp_now();
    if (!bp_registered) { bp_registered = 1; bp_start = now; atexit(bp_write); }
    if (bp_overflow || bp_depth >= BP_STACK) { bp_overflow++; bp_flags |= BP_FLAG_STACK_OVERFLOW; return; }
    uint32_t parent = BP_NONE;
    if (bp_depth > 0) {
        bp_frame *top = &bp_stack[bp_depth - 1];
        parent = top->cur_line != BP_NONE ? top->cur_line : top->node;
    }
    uint32_t node = bp_find(parent, loc, BP_KIND_ROUTINE);
    bp_frame *f = &bp_stack[bp_depth++];
    f->node = node; f->entered = now; f->child_ns = 0;
    f->cur_line = BP_NONE; f->line_start = now; f->line_child_ns = 0;
    if (node != BP_NONE) bp_nodes[node].calls++;
}

void __bruto_prof_exit(void) {
    uint64_t now = bp_now();
    if (bp_overflow) { bp_overflow--; return; }
    if (bp_depth == 0) return;
    bp_frame *f = &bp_stack[--bp_depth];
    bp_close_line(f, now);
    uint64_t total = now - f->entered;
    if (f->node != BP_NONE) {
        bp_node *n = &bp_nodes[f->node];
        n->total_ns += total;
        n->self_ns += total > f->child_ns ? total - f->child_ns : 0;
    }
    if (bp_depth > 0) {
        bp_frame *caller = &bp_stack[bp_depth - 1];
        caller->child_ns += total;
        if (caller->cur_line != BP_NONE) caller->line_child_ns += total;
    }
}

void __bruto_prof_line(uint32_t loc) {
    uint64_t now = bp_now();
    if (bp_overflow || bp_depth == 0) return;
    bp_frame *f = &bp_stack[bp_depth - 1];
    bp_close_line(f, now);
    uint32_t node = bp_find(f->node, loc, BP_KIND_LINE);
    f->cur_line = node;
    f->line_start = now;
    f->line_child_ns = 0;
    if (node != BP_NONE) bp_nodes[node].calls++;
}

static void bp_put32(FILE *fp, uint32_t v) { uint8_t b[4]; for (int i = 0; i < 4; i++) b[i] = (uint8_t)(v >> (8 * i)); fwrite(b, 1, 4, fp); }
static void bp_put64(FILE *fp, uint64_t v) { uint8_t b[8]; for (int i = 0; i < 8; i++) b[i] = (uint8_t)(v >> (8 * i)); fwrite(b, 1, 8, fp); }

static void bp_write(void) {
    uint64_t now = bp_now();
    /* Close frames still open at exit (e.g. halt inside a routine). */
    while (bp_depth > 0) __bruto_prof_exit();
    const char *path = getenv("BRUTO_PROF_OUT");
    if (!path || !*path) return;
    FILE *fp = fopen(path, "wb");
    if (!fp) return;
    /* Node slots are sparse; compact them and remap parent indices. */
    uint32_t *remap = (uint32_t *)malloc(sizeof(uint32_t) * BP_CAPACITY);
    if (!remap) { fclose(fp); return; }
    uint32_t next = 0;
    for (uint32_t i = 0; i < BP_CAPACITY; i++) remap[i] = bp_nodes[i].used ? next++ : BP_NONE;
    fwrite("BPRF", 1, 4, fp);
    bp_put32(fp, 1);
    bp_put32(fp, bp_flags);
    bp_put64(fp, now - bp_start);
    bp_put32(fp, next);
    for (uint32_t i = 0; i < BP_CAPACITY; i++) {
        bp_node *n = &bp_nodes[i];
        if (!n->used) continue;
        fputc(n->kind, fp);
        bp_put32(fp, n->loc);
        bp_put32(fp, n->parent == BP_NONE ? BP_NONE : remap[n->parent]);
        bp_put64(fp, n->calls);
        bp_put64(fp, n->self_ns);
        bp_put64(fp, n->total_ns);
    }
    free(remap);
    fclose(fp);
}
