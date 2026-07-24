# Issue #3: add upstream project acknowledgements

- **Intent:** credit tdlib-purple and Flare as architectural references in the
  README.
- **Actionability:** high; the requested projects and relevant areas of
  influence are clear, with no related pull request or duplicate issue.
- **Priority:** medium; this is a small documentation change that improves
  provenance and contributor trust.
- **Recommendation:** add an `Acknowledgements` section explaining that
  tdlib-purple informed the Purple-facing asynchronous adapter, CMake
  packaging, and testing patterns, while Flare informed the Presage-based
  backend, runtime integration, encrypted storage, and libsecret usage.
- **Wording:** say the design was "informed by" the projects and clarify that
  signal-purple is not a fork of either project.
- **Acceptance criteria:** include canonical project links, name the specific
  areas of influence, and include the not-a-fork clarification.
- **Confidence:** high. The research report and architecture documentation
  identify these precedents, and the repository has no fork lineage.
- **Unknown:** no copied upstream snippets were identified, but that cannot be
  ruled out solely from repository history.
- **Public action:** none; implementation or an issue response requires human
  approval.
