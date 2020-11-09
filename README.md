# Doctave Markdown

This library encapsulates the logic for parsing a Markdown document into HTML in a way that has a
couple Doctave-specific additions.

Currently this means the following:

* A list of subheadings are returned with the generated HTML
* H-tags get associated IDs applied to them so that we can generate links to them
* MermaidJS code snippets get converted into `<div class="mermaid">`
