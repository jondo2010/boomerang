#pragma once

#include "reactor.h"

extern instant_t current_time;
extern interval_t start_time;
extern bool stop_requested;
extern instant_t duration;
extern instant_t stop_time;
extern bool keepalive_specified;

extern interval_t get_elapsed_logical_time();
extern instant_t get_logical_time();
extern instant_t get_physical_time();
extern instant_t get_elapsed_physical_time();

extern pqueue_t *event_q;    // For sorting by time.
extern pqueue_t *reaction_q; // For sorting by deadline.
extern pqueue_t *recycle_q;  // For recycling malloc'd events.
extern pqueue_t *free_q;     // For free malloc'd values carried by events.
extern handle_t __handle;

extern int in_reverse_order(pqueue_pri_t this, pqueue_pri_t that);
extern int event_matches(void *next, void *curr);
extern int reaction_matches(void *next, void *curr);
extern pqueue_pri_t get_event_time(void *a);
extern pqueue_pri_t get_reaction_index(void *a);
extern size_t get_event_position(void *a);
extern size_t get_reaction_position(void *a);
extern void set_event_position(void *a, size_t pos);
extern void set_reaction_position(void *a, size_t pos);
extern void print_reaction(FILE *out, void *reaction);
extern void print_event(FILE *out, void *event);

extern handle_t __schedule(trigger_t *trigger, interval_t extra_delay, void *value);
extern void schedule_output_reactions(reaction_t *reaction);

extern void initialize();