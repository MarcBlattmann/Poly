/* (c) A malloc-heavy arm64 program (ADR 0257): a string-keyed hash table of
 * many small and some large blocks, grown with realloc, half of it freed and
 * refilled in each round, then a merge sort of the survivors. Everything it
 * prints depends only on the contents, never on addresses, so the output is
 * the same wherever it runs. `heap <rounds>`; 6 rounds by default.
 * Written for Poly. */
#include "io.h"

typedef unsigned long size_t;
typedef unsigned long u64;
extern void *malloc(size_t n);
extern void free(void *p);
extern void *realloc(void *p, size_t n);
extern void *calloc(size_t n, size_t m);
extern void *memcpy(void *d, const void *s, size_t n);

#define BUCKETS 4096
#define KEYS 6000

struct node {
    struct node *next;
    char *key;
    u64 *data;
    size_t len;
};

static u64 rng = 0x9e3779b97f4a7c15UL;
static u64 next_rand(void) {
    rng ^= rng << 13;
    rng ^= rng >> 7;
    rng ^= rng << 17;
    return rng;
}

static u64 fnv(const void *p, size_t n, u64 h) {
    const unsigned char *b = p;
    for (size_t i = 0; i < n; i++) h = (h ^ b[i]) * 0x100000001b3UL;
    return h;
}

static char *make_key(u64 k) {
    char *s = malloc(24);
    int n = 0;
    s[n++] = 'k';
    do {
        s[n++] = (char)('a' + k % 26);
        k /= 26;
    } while (k);
    s[n] = 0;
    return s;
}

static size_t keylen(const char *s) {
    size_t n = 0;
    while (s[n]) n++;
    return n;
}

static int keycmp(const char *a, const char *b) {
    while (*a && *a == *b) a++, b++;
    return (unsigned char)*a - (unsigned char)*b;
}

static struct node **table;
static u64 allocations;

static void put(u64 k) {
    char *key = make_key(k);
    u64 h = fnv(key, keylen(key), 0xcbf29ce484222325UL) % BUCKETS;
    for (struct node *n = table[h]; n; n = n->next) {
        if (!keycmp(n->key, key)) {
            /* Grow the existing value with realloc. */
            size_t len = n->len + 1 + next_rand() % 8;
            n->data = realloc(n->data, len * sizeof(u64));
            for (size_t i = n->len; i < len; i++) n->data[i] = k * 31 + i;
            n->len = len;
            free(key);
            allocations++;
            return;
        }
    }
    struct node *n = malloc(sizeof *n);
    n->key = key;
    /* Mostly small values, now and then one past the small classes. */
    n->len = (next_rand() % 64 == 0) ? 700 + next_rand() % 300 : 1 + next_rand() % 30;
    n->data = malloc(n->len * sizeof(u64));
    for (size_t i = 0; i < n->len; i++) n->data[i] = k * 31 + i;
    n->next = table[h];
    table[h] = n;
    allocations += 3;
}

static void drop_half(void) {
    for (int b = 0; b < BUCKETS; b++) {
        struct node **at = &table[b];
        while (*at) {
            struct node *n = *at;
            if (next_rand() & 1) {
                *at = n->next;
                free(n->key);
                free(n->data);
                free(n);
            } else {
                at = &n->next;
            }
        }
    }
}

static void merge_sort(struct node **a, struct node **tmp, size_t n) {
    if (n < 2) return;
    size_t m = n / 2;
    merge_sort(a, tmp, m);
    merge_sort(a + m, tmp, n - m);
    size_t i = 0, j = m, k = 0;
    while (i < m && j < n) tmp[k++] = keycmp(a[i]->key, a[j]->key) <= 0 ? a[i++] : a[j++];
    while (i < m) tmp[k++] = a[i++];
    while (j < n) tmp[k++] = a[j++];
    memcpy(a, tmp, n * sizeof *a);
}

static unsigned long parse(const char *s) {
    unsigned long v = 0;
    while (*s >= '0' && *s <= '9') v = v * 10 + (unsigned long)(*s++ - '0');
    return v;
}

int main(int argc, char **argv) {
    unsigned long rounds = argc > 1 ? parse(argv[1]) : 6;
    table = calloc(BUCKETS, sizeof *table);
    u64 sum = 0;
    size_t live = 0;
    for (unsigned long r = 0; r < rounds; r++) {
        for (u64 i = 0; i < KEYS; i++) put(next_rand() % (KEYS * 2));
        drop_half();
        /* Collect, sort by key and fold the survivors into the checksum. */
        size_t count = 0;
        for (int b = 0; b < BUCKETS; b++)
            for (struct node *n = table[b]; n; n = n->next) count++;
        struct node **all = malloc(count * sizeof *all);
        struct node **tmp = malloc(count * sizeof *tmp);
        size_t k = 0;
        for (int b = 0; b < BUCKETS; b++)
            for (struct node *n = table[b]; n; n = n->next) all[k++] = n;
        merge_sort(all, tmp, count);
        for (size_t i = 0; i < count; i++) {
            sum = fnv(all[i]->key, keylen(all[i]->key), sum);
            sum = fnv(all[i]->data, all[i]->len * sizeof(u64), sum);
        }
        free(all);
        free(tmp);
        live = count;
    }
    say("a64-heap: ");
    say_num(rounds);
    say(" rounds, ");
    say_num(allocations);
    say(" allocations, ");
    say_num(live);
    say(" live, checksum ");
    say_hex(sum);
    say("\n");
    say("a64-heap: done\n");
    return 0;
}
