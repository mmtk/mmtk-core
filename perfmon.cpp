#include <err.h>
#include <cstring>
#include <string>
#include <limits>
#include <iostream>
#include <vector>
#include <memory>
#include <perfmon/pfmlib_perf_event.h>

#define ASSERT(condition, ...)   \
    if (!(condition)) {          \
        errx(1, __VA_ARGS__);    \
        exit(-1);                \
    }

struct PerfEventData {
    long long raw_count;
    long long time_enabled;
    long long time_running;
    constexpr bool IsOverflowed() const {
        return raw_count < 0 || time_enabled < 0 || time_running < 0;
    }
    constexpr bool IsContended() const {
        return time_enabled == 0;
    }
    constexpr double operator - (const PerfEventData& prev) const {
        if (IsOverflowed() || prev.IsOverflowed() || IsContended() || prev.IsContended())
            return std::numeric_limits<double>::quiet_NaN();
        return (double) (raw_count - prev.raw_count);
    }
};

class PerfEvents {
public:
    const std::vector<std::string> events;

    PerfEvents() {}
    PerfEvents(std::initializer_list<std::string> events): events(events) {}
    PerfEvents(const std::vector<std::string>& events): events(events) {}
    PerfEvents(std::vector<std::string>&& events): events(events) {}

    void Prepare() {
        perf_event_fds_.resize(events.size());
        perf_event_attrs_.resize(events.size());
        pfm_prepare();
        for (size_t i = 0; i < events.size(); i++) {
            pfm_create(i, events[i].c_str());
        }
    }

    void Enable() {
        pfm_enable();
    }

    std::vector<PerfEventData> ReadAll() const {
        std::vector<PerfEventData> results(events.size());
        for (size_t i = 0; i < events.size(); i++) {
            pfm_read(i, &results[i]);
        }
        return results;
    }
private:
    std::vector<int> perf_event_fds_;
    std::vector<struct perf_event_attr> perf_event_attrs_;
    bool initialized_ = false;

    void pfm_prepare() {
        const int ret = pfm_initialize();
        ASSERT(ret == PFM_SUCCESS, "error in pfm_initialize: %s", pfm_strerror(ret));
        for(size_t i = 0; i < events.size(); i++) {
            perf_event_attrs_[i].size = sizeof(struct perf_event_attr);
        }
        initialized_ = true;
    }

    void pfm_create(size_t id, const char* event_name) {
        struct perf_event_attr* pe = &perf_event_attrs_[id];
        const int ret = pfm_get_perf_event_encoding(event_name, PFM_PLM3, pe, NULL, NULL);
        ASSERT(ret == PFM_SUCCESS, "error creating event %zu '%s': %s\n", id, event_name, pfm_strerror(ret));
        pe->read_format = PERF_FORMAT_TOTAL_TIME_ENABLED | PERF_FORMAT_TOTAL_TIME_RUNNING;
        pe->disabled = 1;
        pe->inherit = 1;
        perf_event_fds_[id] = perf_event_open(pe, 0, -1, -1, 0);
        ASSERT(perf_event_fds_[id] != -1, "error in perf_event_open for event %zu '%s'", id, event_name);
        const auto enabled = ioctl(perf_event_fds_[id], PERF_EVENT_IOC_ENABLE, 0);
        ASSERT(enabled != -1, "error in ioctl(enable) for event %zu '%s'", id, event_name);
    }

    void pfm_enable() {
        ASSERT(initialized_, "perf events is not initialized");
        const int ret = prctl(PR_TASK_PERF_EVENTS_ENABLE);
        ASSERT(ret == 0, "error in prctl(PR_TASK_PERF_EVENTS_ENABLE)");
    }

    void pfm_disable() {
        ASSERT(initialized_, "perf events is not initialized");
        const int ret = prctl(PR_TASK_PERF_EVENTS_DISABLE);
        ASSERT(ret == 0, "error in prctl(PR_TASK_PERF_EVENTS_DISABLE)");
    }

    inline void pfm_read(size_t id, PerfEventData* values) const {
        const size_t expected_bytes = 3 * sizeof(long long);
        const int ret = read(perf_event_fds_[id], values, expected_bytes);
        ASSERT(ret >= 0, "error reading event: %s", strerror(errno));
        ASSERT((size_t) ret == expected_bytes, "read of perf event did not return 3 64-bit values");
        ASSERT(values->time_enabled == values->time_running, "perf event counter was scaled");
    }
};

static std::unique_ptr<PerfEvents> perf_events;
static std::vector<PerfEventData> initial_counters;

typedef struct {
    const char* name;
    uint64_t value;
} EventResult;

static std::vector<EventResult> results;

extern "C" {
    __attribute__ ((visibility ("default"))) void mmtk_perf_harness_prepare() {
        // printf("mmtk_perf_harness_prepare\n");
        std::vector<std::string> events;// = { "PERF_COUNT_HW_CACHE_DTLB:MISS" };
        auto event_names = std::getenv("PERF_EVENTS");
        ASSERT(event_names, "env PERF_EVENTS not set");
        auto token = strtok(event_names, ",");
        while(token != nullptr) {
            events.push_back(token);
            token = strtok(nullptr, ",");
        }
        perf_events = std::make_unique<PerfEvents>(events);
        perf_events->Prepare();
    }

    __attribute__ ((visibility ("default"))) void mmtk_perf_harness_begin() {
        // printf("mmtk_perf_harness_begin\n");
        perf_events->Enable();
        initial_counters = perf_events->ReadAll();
    }

    __attribute__ ((visibility ("default"))) const EventResult* mmtk_perf_harness_end(int* size) {
        // printf("mmtk_perf_harness_end\n");
        auto counters = perf_events->ReadAll();
        for (size_t i = 0; i < perf_events->events.size(); i++) {
            EventResult result;
            result.name = perf_events->events[i].c_str();
            result.value = uint64_t(counters[i] - initial_counters[i]);
            results.push_back(result);
        }
        *size = results.size();
        return results.data();
    }
}