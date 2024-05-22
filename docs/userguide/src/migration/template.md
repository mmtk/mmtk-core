<!--hidden-->
<!-- We use `mdbook-hide` to hide this document from the published version. -->

# The template of the API migration guide

This template is for the developers of MMTk-core.


## View control

The following buttons allow the readers to choose how many details they want to read.  Therefore, we
can keep the TL;DR and the first two levels of the lists terse, and add detailed explanations at the
third level.

Try clicking those buttons and see what they do.

{{#include ../../assets/snippets/view-controls.html}}

<div id="api-migration-detail-body"><!-- We use JavaScript to process things within this div. -->

## 0.xx.0

### Title of a change

```admonish tldr
Use a few sentences to summarize the change so that the reader knows what has been changed without
reading through the following list.

Keep in mind that the reader can use the buttons above to hide everything but the TL;DR.  Make sure
the TL;DR part covers all types/modules that are changed so that the readers know what changed by
reading the TL;DR alone.
```

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
*   type `crate::policy::immix::Block` *(Note: Qualify the name if ambiguous)*
    -   insert methods here...
*   type `crate::policy::marksweepspace::native_ms::Block` *(Note: ditto)*
    -   insert methods here...
*   trait `Bar`
    -   `method1()`
        +   **Only affects users of feature "yyyy"** *(Note: When omitted, it affects everyone.)*
        +   MMTk now expects the VM binding to...
        +   The VM binding should...
    -   `method2()`
        +   MMTk now expects the VM binding to...
        +   The VM binding should...
*   trait `Baz`
    -   insert methods here...

Not API change, but worth noting:

*   Add other things besides API changes that need the attention from the VM binding developers.

See also:

-   PR: <https://github.com/mmtk/mmtk-core/pull/121>
-   PR: <https://github.com/mmtk/mmtk-core/pull/122>
-   Example: <https://github.com/mmtk/mmtk-openjdk/pull/42>
-   Example: <https://github.com/mmtk/mmtk-openjdk/pull/43>

### Title of another change

```admonish tldr
Insert summary here.

**Only affects users of feature "zzzz"** *(Note: When omitted, it affects everyone.)*

```

API changes:

*   trait `Bar2`
    -   `method3()`
    -   `method4()`

See also:

-   PR: <https://github.com/mmtk/mmtk-core/pull/123>
-   Example: <https://github.com/mmtk/mmtk-openjdk/pull/44>

## 0.yy.0

### Title of an old change

```admonish tldr
Insert summary here.
```

API changes:

*   type `Foo3`
    -   `bar()`
    -   `baz()`

See also:

-   PR: <https://github.com/mmtk/mmtk-core/pull/124>
-   Example: <https://github.com/mmtk/mmtk-openjdk/pull/45>

</div>

<script type="text/javascript">
// This will tell api-migration-details.js to run some code and enable the collapsing feature.
const isApiMigrationGuide = true;
</script>

<!--
vim: tw=100
-->
