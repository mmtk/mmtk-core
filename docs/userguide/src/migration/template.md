# The template of the API migration guide

This template is for the developers of MMTk-core.


## View control

The following buttons allow the third level of the lists to be collapsed.  Therefore, you can keep
the first two levels of the lists terse, and add detailed explanations at the third level.

Try clicking those buttons and see what they do.

<button class="api-migration-details-collapse-all" type="button">Collapse all details</button>
<button class="api-migration-details-expand-all" type="button">Expand all details</button>


## 0.xx.0

### Title of a change

**TL;DR** Use a few sentences to summarize the change so that the reader knows what has bee changed
without reading through the following list.

API changes:

*   type `Foo`
    -   `abc()` is removed.
        +   Use `abc2()` instead.
        +   *(Note 1: Put types/modules/... on the first level.  Keep the prefix "type", "module",
            ...  before `Foo` so that readers can search for "type Foo" to find actual changes on
            `Foo` instead of places that merely mention `Foo`.)*
        +   *(Note 2: Put functions/methods/constants/... on the second level.)*
        +   *(Note 3: Put suggestions and more details on the third level.)*
    -   `defg()`
        +   It now takes a new argument `foobarbaz` which requires blah blah...
        +   And you need to blah blah blah blah blah blah blah blah blah blah blah blah blah blah
            blah blah blah blah blah blah blah blah blah blah blah blah blah blah blah blah blah
            blah blah blah blah blah blah blah blah blah blah blah blah blah blah blah blah...
        +   *(Note 1: If the change is more complicated than "... is removed", feel free to put it
            down one level.)*
        +   *(Note 2: Since the third level is collapsible, feel free to add more details.)*
*   module `aaa::bbb::ccc`
    -   **Only affects users of feature "xxxx"** *(Note: When omitted, it affects everyone.)*
    -   `method1()`
        +   What happened to it...
        +   Suggestions...
    -   `method2()`
        +   What happened to it...
        +   Suggestions...

VM bindings need to re-implement the following traits:

*   trait `Bar`
    -   `method1()`
        +   MMTk now expects the VM binding to...
        +   The VM binding should...
    -   `method2()`
        +   MMTk now expects the VM binding to...
        +   The VM binding should...
*   trait `Baz`
    -   insert methods here...

Miscellaneous changes:

*   Add more stuff if it doesn't belong to any of the categories, but still needs the attention from
    the VM binding developers.

See also:

-   PR: <https://github.com/mmtk/mmtk-core/pull/121>
-   PR: <https://github.com/mmtk/mmtk-core/pull/122>
-   Example: <https://github.com/mmtk/mmtk-openjdk/pull/42>
-   Example: <https://github.com/mmtk/mmtk-openjdk/pull/43>


<!--
vim: tw=100
-->
