/* sim_ci_main.cpp — Headless CI harness for liero_sim.
 *
 * Runs a deterministic game simulation for N frames and dumps a CRC per frame.
 * Optionally compares against a golden CRC file to detect regressions.
 *
 * Usage:
 *   sim_ci --tc <tc_dir> [--frames N] [--dump-crcs <out.csv>] [--golden <golden.csv>]
 *
 * The simulation uses zero inputs (no player input) and a fixed random seed,
 * producing a deterministic sequence of checksums that must match across builds.
 */

#include "../game/sim_c_api.h"

#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <fstream>
#include <string>
#include <vector>

static void usage(const char* prog)
{
    std::fprintf(stderr,
        "Usage: %s --tc <dir> [--frames N] [--dump-crcs <file>] [--golden <file>]\n",
        prog);
}

int main(int argc, char* argv[])
{
    const char* tcPath    = nullptr;
    const char* dumpPath  = nullptr;
    const char* goldenPath = nullptr;
    int         frames    = 1000;
    int         numWorms  = 2;

    for (int i = 1; i < argc; ++i)
    {
        if (std::strcmp(argv[i], "--tc") == 0 && i + 1 < argc)
            tcPath = argv[++i];
        else if (std::strcmp(argv[i], "--frames") == 0 && i + 1 < argc)
            frames = std::atoi(argv[++i]);
        else if (std::strcmp(argv[i], "--dump-crcs") == 0 && i + 1 < argc)
            dumpPath = argv[++i];
        else if (std::strcmp(argv[i], "--golden") == 0 && i + 1 < argc)
            goldenPath = argv[++i];
        else if (std::strcmp(argv[i], "--num-worms") == 0 && i + 1 < argc)
            numWorms = std::atoi(argv[++i]);
        else
        {
            std::fprintf(stderr, "Unknown argument: %s\n", argv[i]);
            usage(argv[0]);
            return 1;
        }
    }

    if (!tcPath)
    {
        usage(argv[0]);
        return 1;
    }

    SimHandle* sim = (numWorms == 2) ? sim_create(tcPath) : sim_create_n(tcPath, numWorms);
    if (!sim)
    {
        std::fprintf(stderr, "sim_ci: failed to create simulation from TC '%s'\n", tcPath);
        return 1;
    }

    sim_start_game(sim, /*seed=*/42);

    // Open CRC output file if requested
    std::ofstream out;
    if (dumpPath)
    {
        out.open(dumpPath);
        if (!out.is_open())
        {
            std::fprintf(stderr, "sim_ci: cannot open output file '%s'\n", dumpPath);
            sim_destroy(sim);
            return 1;
        }
    }

    // Load golden CRCs for comparison
    std::vector<std::pair<int,uint64_t>> golden;
    if (goldenPath)
    {
        std::ifstream gf(goldenPath);
        if (!gf.is_open())
        {
            std::fprintf(stderr, "sim_ci: cannot open golden file '%s'\n", goldenPath);
            sim_destroy(sim);
            return 1;
        }
        std::string line;
        while (std::getline(gf, line))
        {
            int cycle = 0;
            uint64_t crc = 0;
            if (std::sscanf(line.c_str(), "%d,%llu", &cycle, &crc) == 2)
                golden.push_back({cycle, crc});
        }
    }

    // Run simulation
    sim_frame_input_t inputs = {};
    inputs.num_worms = 0; // no input — worms stand still

    int mismatches = 0;
    std::size_t goldenIdx = 0;

    for (int f = 0; f < frames; ++f)
    {
        sim_step(sim, &inputs);
        int   cycle = sim_cycles(sim);
        uint64_t crc = sim_checksum(sim);

        if (out.is_open())
            out << cycle << "," << crc << "\n";

        // Compare against golden if provided
        if (!golden.empty() && goldenIdx < golden.size())
        {
            if (golden[goldenIdx].first == cycle)
            {
                if (golden[goldenIdx].second != crc)
                {
                    std::fprintf(stderr,
                        "sim_ci: MISMATCH at frame %d: expected %llu got %llu\n",
                        cycle,
                        (unsigned long long)golden[goldenIdx].second,
                        (unsigned long long)crc);
                    ++mismatches;
                }
                ++goldenIdx;
            }
        }

        if (sim_is_game_over(sim))
            break;
    }

    sim_destroy(sim);

    if (mismatches > 0)
    {
        std::fprintf(stderr, "sim_ci: %d CRC mismatches detected — FAIL\n", mismatches);
        return 1;
    }

    std::printf("sim_ci: %d frames OK\n", frames);
    return 0;
}
