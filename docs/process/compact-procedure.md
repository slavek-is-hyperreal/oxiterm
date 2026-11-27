# Compact procedure (long lead sessions)

Use this when the lead's conversation context runs low (owner request, 2026-11-27). Goal: after `/compact` the new context knows every thread, its state, and where to read the full discussion in the old transcript.

1. Find the transcript: `~/.claude/projects/<project>/<session-id>.jsonl` (local, private; never commit it or quote its path in public docs).
2. List the threads since the last compact: one line each, with a distinctive phrase from the owner's first message of the thread.
3. Get line numbers: for each phrase, `grep -n -F "<phrase>" <transcript> | awk -F: '$1>=<line of the last compact>{print $1; exit}'` (first occurrence after the last compact).
4. Write the `/compact` prompt (in the owner's language), with:
   - the transcript path and how to read a fragment (`sed -n 'START,+200p' <file>`, `grep -n -F "<phrase>" <file>`);
   - one row per thread: number, title, start line(s), grep phrase, 1-3 sentences of state and owner decisions, artefacts (commits, files, branches);
   - a "work in progress at compact time" list: every running agent/Gemini session, branch/worktree, gate, server, and what to do next;
   - the standing rules (language, models per role, BAD ACTOR, gates, privacy, budget) or a pointer to where they are recorded.
5. Give the prompt to the owner to paste after `/compact `.
6. After the compact, first action: check running background work and continue from the "in progress" list.
