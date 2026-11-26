Short desc: Research report: three static site generators, as a PDF
Research task for the repository `research-notes` (no code: Markdown sources, a `Makefile` and the built PDFs). The
repository already has what this task uses: `make csv-check FILE=<file>` (checks a CSV of facts), `tools/csv2md` (turns
the CSV into a Markdown table), `make table`, `make pdf` (pandoc with the template and PDF engine set in the
`Makefile`), `make links` (checks every URL of `sources/*.csv`) and `make check` (all of them). Two stages, three items,
one pull request per item, in the order of their `Depends on:` lines. Every decision about the content is made below:
do not add other projects, other criteria or another format. If a fact cannot be found as described, write it as
`unknown` with what you searched, and go on; ask the human in your questions file only when a whole criterion cannot
be filled for two projects or more, or when a tool above is missing or fails.

Labels used in this file:
- `Short desc:` — one line of at most 70 characters that `agent status` and the notifications show; only at the top.
  There is no `Allow:` line: this task changes no test and no protected path, so the checks before the review have
  nothing to ask.
- `1.`, `2.` — a stage of the work; `1.1`, `1.2` — an item of it, one pull request each. `[ ]` marks an item not done.
- `Depends on:` — under a stage: the stages that must be merged before it starts. Under an item (indented): the items
  it builds on, for the team's plan.
- `Expected result:` — what the reviewers check the pull request against. `Checks:` — what anyone can run or look at to
  see that the item is done. Both alone on their line, at the indentation of the item's text.
- `W1`, `W2`, ... — a worker step: a small, self-contained piece of an item that the lead hands to a worker with
  `delegate`. The number only names the step; the order is given by "after W4" and `‖`.
- `‖` — the steps on both sides may run at the same time, each by its own worker.
- `By: worker` — the mark the lead puts on such a step in its plan of steps.
- `A1`–`A8` — the basic facts of a project, collected in item 1.1 (language, licence, releases, stars, ...).
- `B1`–`B4` — how a project is used in practice, collected in item 1.2 (build time, templating, themes, community).
  The letter is the group, the number the criterion; the same id names the row in `sources/facts.csv` (column `row`),
  so "rows A1–A8" means one row per criterion for each project.
- `K1`, `K2`, ... — a check of an item (K for "Kontrola", check): a condition that must hold before the item is handed
  over, numbered within each item.

Why: the human must choose a static site generator for a documentation site of about 300 pages and wants a short,
sourced comparison of Hugo, Jekyll and Eleventy (11ty) that can be printed and passed around: at most 8 pages of A4.

Rules for every item:
- Only primary sources where they exist: the project's own website and documentation, its repository on GitHub, its
  release notes, its licence file, its package registry page (npm, RubyGems, pkg.go.dev). Blogs, forums and
  comparison sites only for the criterion "Community view" (B4), at most three per project, each marked as such.
- Every fact in `sources/facts.csv` has a source URL and the date it was read (`2026-11-26`). A number without a source
  is not written.
- Numbers that change (stars, releases, open issues) are read on the same day for all three projects, and the report
  says "as of 26.11.2026".
- Quote at most two sentences from any one source, in quotation marks, with the source; everything else in your own
  words.
- No personal data: no names of maintainers beyond what the project's own page shows, no e-mail addresses.
- No account, login or paid service: only pages anyone can open. Fetch politely: one request at a time per site.
- The CSV format, the same in every file: a header line `project,row,criterion,value,unit,url,read_on,note`; `project`
  is `hugo`, `jekyll` or `eleventy`; `read_on` is `YYYY-MM-DD`; `value` is `unknown` with a `note` when not found;
  `note` is `community` for a community source.
- `make check` passes on the commit you hand over. No new tool is installed.
- No versions to raise: this repository has none. The PR description lists what was done, the output of the item's
  checks and every `unknown` with its reason.

Workers: give the steps below to workers with `delegate` and mark them `By: worker` in your plan. Each step is
self-contained: give the worker the text of the step, the criteria list of its item and the CSV format, nothing more.
A research step covers one project and one group of criteria, writes rows only into its own file
`sources/<project>-facts.csv` (group A) or `sources/<project>-practice.csv` (group B), and ends with
`make csv-check FILE=<its file>`. You merge the files, check every number yourself against its source (open the URL;
a second worker's reading is not enough), write the report's text and build the PDF. Steps joined by ‖ may run at the
same time.

- W1 ‖ W2 ‖ W3 (item 1.1, the facts):
  - W1: Hugo — the rows A1–A8 of 1.1 into `sources/hugo-facts.csv`.
  - W2: Jekyll — the same rows into `sources/jekyll-facts.csv`.
  - W3: Eleventy — the same rows into `sources/eleventy-facts.csv`.
  - Check of each: `make csv-check FILE=<file>` (8 rows, every row has `url` and `read_on`, no empty `value`).
- W4 ‖ W5 ‖ W6 (item 1.2, after item 1.1 is merged): for one project each, the rows B1–B4 of 1.2 into
  `sources/<project>-practice.csv`. Check: `make csv-check FILE=<file>` (4 rows).
- W7 (item 1.2, after W4–W6): `make links > sources/link-report.txt`: every URL of `sources/*.csv` once, with its
  HTTP status, and the URLs that redirect to another domain. Do not change the CSV files. Check: the count of lines
  equals the count of distinct URLs.
- W8 ‖ W9 (item 2.1):
  - W8: the comparison table `report/table.md`, generated by `make table` from `sources/facts.csv`; no number typed by
    hand. Check: `make table` run again leaves no diff.
  - W9: the bibliography `report/sources.md`: every URL of `sources/facts.csv` once, numbered in the order of first
    use in the table, with the page title and "read on 26.11.2026". Check: the count of entries equals the count of
    distinct URLs.
- W10 (last of 2.1, after the lead's text): build the PDF with `make pdf`, run the checks K3–K6 of 2.1, and paste their
  output into the PR description.

Not in this task: a recommendation of one generator (the human decides); hosting and CI; performance measured by us
(only published figures, with their source); themes and plugins beyond the count in B3.

Estimate (mine + 50%): 1.1 — 24 rows, about 2 h of research; 1.2 — 12 rows and a link report, about 1.5 h; 2.1 — a
report of 6–8 pages, about 2 h.

1. The facts, sourced
Depends on: none
[ ] 1.1 Basic facts of each project in one CSV file
    Depends on: none
    Expected result:
    - `sources/facts.csv`, in the CSV format of the rules, one row per project and criterion, 24 rows:
      - A1 implementation language; A2 licence (SPDX id, from the licence file); A3 first public release (year, from
        the earliest tag or release); A4 latest stable release (version and date); A5 GitHub stars; A6 open issues;
      - A7 releases in the last 12 months (count of release tags dated 26.11.2025–26.11.2026);
      - A8 how it is installed (the package registry or binary download the docs name first).
    - The three worker files merged into it, sorted by project and row; the worker files stay in `sources/`.
    - Checks of the content, done by the lead:
      - every `url` opens and shows the value;
      - A5 and A6 read on the same day for all three projects;
      - A2 is an SPDX id (`Apache-2.0`, `MIT`, ...), not a sentence;
      - A4's date is not after 26.11.2026 and A3's year is not after A4's.
    Checks:
    - K1. `make csv-check FILE=sources/facts.csv` passes: 24 rows, no row without `url` and `read_on`.
    - K2. The PR description lists every `unknown` with what was searched.
    - K3. `make check` passes.
[ ] 1.2 How each project is used in practice
    Depends on: 1.1
    Expected result:
    - `sources/facts.csv` gains 12 rows:
      - B1 build time of a site of about 1000 pages, as the project or a benchmark it links to publishes it (with the
        hardware it names); `unknown` if none is published;
      - B2 the templating language; B3 the number of official or listed themes (from the project's own theme page);
      - B4 "Community view": one sentence in your own words summing up at most three community sources, their URLs in
        `url` (separated by spaces), `community` in `note`.
    - `sources/link-report.txt` from W7. Every dead link is replaced by a working source for the same fact, or the
      value becomes `unknown` with a note; the report is made again after the replacements.
    Checks:
    - K1. `make csv-check FILE=sources/facts.csv` passes: 36 rows.
    - K2. `sources/link-report.txt` lists no dead link that is still in `facts.csv`.
    - K3. No B1 value is given without the hardware in its `note`.
    - K4. `make check` passes.

2. The report
Depends on: 1
[ ] 2.1 `report/ssg-comparison.pdf`, at most 8 pages of A4
    Depends on: 1.2
    Expected result:
    - `report/ssg-comparison.md`, assembled by `make pdf` with `report/table.md` and `report/sources.md`, in this order:
      1. "Summary": the title, "as of 26.11.2026", and at most 120 words: what was compared and the three facts that
         differ most. No recommendation.
      2. "Method": the criteria A1–A8 and B1–B4, the kinds of sources, what `unknown` means.
      3. "Comparison": the table (`report/table.md`), every number with its bibliography number in brackets, e.g.
         `[4]`.
      4. "Hugo", "Jekyll", "Eleventy": one section per project, 150–250 words each, in the same order of topics: what
         it is, how a site is built, what stands out in the facts, what is `unknown`.
      5. "Limits": numbers change, published benchmarks use other hardware, the community view is three sources and
         not a survey.
      6. "Sources" (`report/sources.md`).
    - `report/ssg-comparison.pdf` built from it by `make pdf`: A4, page numbers, the table on one page.
    - Both files committed; the PDF is rebuilt in the last commit so that it matches the Markdown.
    Checks:
    - K1. Every number in the report text equals its value in `sources/facts.csv` (the lead checks them one by one and
      lists them in the PR description).
    - K2. No bracket number in the text points past the last entry of the sources; every source is cited at least once.
    - K3. `pdfinfo report/ssg-comparison.pdf` shows `Pages:` 8 or fewer and `Page size:` A4.
    - K4. `pdftotext report/ssg-comparison.pdf -` contains `as of 26.11.2026` and the section titles "Summary",
      "Method", "Comparison", "Hugo", "Jekyll", "Eleventy", "Limits", "Sources".
    - K5. The word counts of the summary (at most 120) and of each project section (150–250), from `wc -w` on the
      Markdown, are in the PR description.
    - K6. `grep -inE 'we recommend|you should (use|choose)|@[a-z0-9-]+\.[a-z]' report/ssg-comparison.md` finds
      nothing: no recommendation, no e-mail address.
    - K7. `make check` passes.
