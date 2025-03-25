function setDetailsExpanded(level0Expanded, level3Expanded) {
    document.querySelectorAll("#api-migration-detail-body details.api-migration-level0-detail").forEach((value) => {
        value.open = level0Expanded;
    });
    document.querySelectorAll("#api-migration-detail-body details.api-migration-level3-detail").forEach((value) => {
        value.open = level3Expanded;
    });
}

function makeDetails(title, cssClass) {
    let details = document.createElement("details");
    details.classList.add(cssClass);
    details.open = true;
    let summary = document.createElement("summary");
    summary.innerText = title;
    details.appendChild(summary);
    return details;
}

function wrapWithDetails(elem) {
    let details = makeDetails("show details...", "api-migration-level3-detail")
    elem.replaceWith(details);
    details.appendChild(elem);
}

function isH3OrAbove(node) {
    let nodeName = node.nodeName;
    return nodeName == "H1" || nodeName == "H2" || nodeName == "H3";

}

function wrapAfterTldr() {
    document.querySelectorAll("#api-migration-detail-body div.admonition").forEach((value, key, parent) => {
        if (value.id.startsWith("admonition-tldr")) {
            let details = makeDetails("show details...", "api-migration-level0-detail")
            value.insertAdjacentElement("afterend", details);
            while (details.nextSibling != null && !isH3OrAbove(details.nextSibling)) {
                details.appendChild(details.nextSibling);
            }
        }
    });
}

function doApiMigrationGuide() {
    document.querySelectorAll("#api-migration-detail-body ul ul ul").forEach((value) => {
        wrapWithDetails(value);
    });

    wrapAfterTldr();

    document.querySelectorAll(".api-migration-details-show-tldr").forEach((value) => {
        value.addEventListener("click", (e) => setDetailsExpanded(false, false));
    });

    document.querySelectorAll(".api-migration-details-show-outline").forEach((value) => {
        value.addEventListener("click", (e) => setDetailsExpanded(true, false));
    });

    document.querySelectorAll(".api-migration-details-show-all").forEach((value) => {
        value.addEventListener("click", (e) => setDetailsExpanded(true, true));
    });
}

// Only run the code if the current page is labelled as a migration guide.
if (typeof isApiMigrationGuide !== "undefined" && isApiMigrationGuide) {
    doApiMigrationGuide();
}
