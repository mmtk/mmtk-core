function setDetailsExpanded(expanded) {
    document.querySelectorAll("details").forEach((value) => {
        value.open = expanded;
    });
}

function wrapWithDetails(elem) {
    let details = document.createElement("details");
    details.open = true;
    let summary = document.createElement("summary");
    summary.innerText = "show details...";
    details.appendChild(summary);
    elem.replaceWith(details);
    details.appendChild(elem);
}

function doApiMigrationGuide() {
    document.querySelectorAll("ul ul ul").forEach((value) => {
        wrapWithDetails(value);
    });

    document.querySelectorAll(".api-migration-details-collapse-all").forEach((value) => {
        value.addEventListener("click", (e) => setDetailsExpanded(false));
    });

    document.querySelectorAll(".api-migration-details-expand-all").forEach((value) => {
        value.addEventListener("click", (e) => setDetailsExpanded(true));
    });
}

// Only run the code if the current page is labelled as a migration guide.
if (typeof isApiMigrationGuide !== "undefined" && isApiMigrationGuide) {
    doApiMigrationGuide();
}
