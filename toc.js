// Populate the sidebar
//
// This is a script, and not included directly in the page, to control the total size of the book.
// The TOC contains an entry for each page, so if each page includes a copy of the TOC,
// the total size of the page becomes O(n**2).
class MDBookSidebarScrollbox extends HTMLElement {
    constructor() {
        super();
    }
    connectedCallback() {
        this.innerHTML = '<ol class="chapter"><li class="chapter-item expanded affix "><a href="index.html">Introduction</a></li><li class="chapter-item expanded affix "><a href="glossary.html">Glossary</a></li><li class="chapter-item expanded affix "><li class="part-title">For GC Developers</li><li class="chapter-item expanded "><a href="tutorial/prefix.html"><strong aria-hidden="true">1.</strong> Tutorial: Add a new GC plan to MMTk</a></li><li><ol class="section"><li class="chapter-item expanded "><div><strong aria-hidden="true">1.1.</strong> Introduction</div></li><li><ol class="section"><li class="chapter-item expanded "><a href="tutorial/intro/what_is_mmtk.html"><strong aria-hidden="true">1.1.1.</strong> What is MMTk?</a></li><li class="chapter-item expanded "><a href="tutorial/intro/what_will_this_tutorial_cover.html"><strong aria-hidden="true">1.1.2.</strong> What will this tutorial cover?</a></li><li class="chapter-item expanded "><a href="tutorial/intro/glossary.html"><strong aria-hidden="true">1.1.3.</strong> Glossary</a></li></ol></li><li class="chapter-item expanded "><div><strong aria-hidden="true">1.2.</strong> Preliminaries</div></li><li><ol class="section"><li class="chapter-item expanded "><a href="tutorial/preliminaries/set_up.html"><strong aria-hidden="true">1.2.1.</strong> Set up MMTk and OpenJDK</a></li><li class="chapter-item expanded "><a href="tutorial/preliminaries/test.html"><strong aria-hidden="true">1.2.2.</strong> Test the build</a></li></ol></li><li class="chapter-item expanded "><div><strong aria-hidden="true">1.3.</strong> MyGC</div></li><li><ol class="section"><li class="chapter-item expanded "><a href="tutorial/mygc/create.html"><strong aria-hidden="true">1.3.1.</strong> Create MyGC</a></li><li class="chapter-item expanded "><a href="tutorial/mygc/ss/prefix.html"><strong aria-hidden="true">1.3.2.</strong> Building a semispace GC</a></li><li><ol class="section"><li class="chapter-item expanded "><a href="tutorial/mygc/ss/alloc.html"><strong aria-hidden="true">1.3.2.1.</strong> Allocation</a></li><li class="chapter-item expanded "><a href="tutorial/mygc/ss/collection.html"><strong aria-hidden="true">1.3.2.2.</strong> Collection</a></li><li class="chapter-item expanded "><a href="tutorial/mygc/ss/exercise.html"><strong aria-hidden="true">1.3.2.3.</strong> Exercise</a></li><li class="chapter-item expanded "><a href="tutorial/mygc/ss/exercise_solution.html"><strong aria-hidden="true">1.3.2.4.</strong> Exercise solution</a></li></ol></li><li class="chapter-item expanded "><a href="tutorial/mygc/gencopy.html"><strong aria-hidden="true">1.3.3.</strong> Building a generational copying GC</a></li></ol></li><li class="chapter-item expanded "><a href="tutorial/further_reading.html"><strong aria-hidden="true">1.4.</strong> Further Reading</a></li></ol></li><li class="chapter-item expanded "><li class="part-title">For Language Runtime Developers</li><li class="chapter-item expanded "><a href="portingguide/prefix.html"><strong aria-hidden="true">2.</strong> Porting Guide</a></li><li><ol class="section"><li class="chapter-item expanded "><a href="portingguide/portability.html"><strong aria-hidden="true">2.1.</strong> MMTk’s Approach to Portability</a></li><li class="chapter-item expanded "><a href="portingguide/before_start.html"><strong aria-hidden="true">2.2.</strong> Before Starting a Port</a></li><li class="chapter-item expanded "><a href="portingguide/howto/prefix.html"><strong aria-hidden="true">2.3.</strong> How to Undertake a Port</a></li><li><ol class="section"><li class="chapter-item expanded "><a href="portingguide/howto/nogc.html"><strong aria-hidden="true">2.3.1.</strong> NoGC</a></li><li class="chapter-item expanded "><a href="portingguide/howto/next_steps.html"><strong aria-hidden="true">2.3.2.</strong> Next Steps</a></li></ol></li><li class="chapter-item expanded "><a href="portingguide/debugging/prefix.html"><strong aria-hidden="true">2.4.</strong> Debugging Tips</a></li><li><ol class="section"><li class="chapter-item expanded "><a href="portingguide/debugging/assertions.html"><strong aria-hidden="true">2.4.1.</strong> Enabling Debug Assertions</a></li><li class="chapter-item expanded "><a href="portingguide/debugging/print_obj_info.html"><strong aria-hidden="true">2.4.2.</strong> Print Object Info</a></li><li class="chapter-item expanded "><a href="portingguide/debugging/immix.html"><strong aria-hidden="true">2.4.3.</strong> Copying in Immix</a></li></ol></li><li class="chapter-item expanded "><a href="portingguide/perf_tuning/prefix.html"><strong aria-hidden="true">2.5.</strong> Performance Tuning</a></li><li><ol class="section"><li class="chapter-item expanded "><a href="portingguide/perf_tuning/lto.html"><strong aria-hidden="true">2.5.1.</strong> Link Time Optimization</a></li><li class="chapter-item expanded "><a href="portingguide/perf_tuning/alloc.html"><strong aria-hidden="true">2.5.2.</strong> Optimizing Allocation</a></li></ol></li><li class="chapter-item expanded "><a href="portingguide/concerns/prefix.html"><strong aria-hidden="true">2.6.</strong> VM-specific Concerns</a></li><li><ol class="section"><li class="chapter-item expanded "><a href="portingguide/concerns/weakref.html"><strong aria-hidden="true">2.6.1.</strong> Finalizers and Weak References</a></li><li class="chapter-item expanded "><a href="portingguide/concerns/address-based-hashing.html"><strong aria-hidden="true">2.6.2.</strong> Address-based Hashing</a></li></ol></li></ol></li><li class="chapter-item expanded "><a href="migration/prefix.html"><strong aria-hidden="true">3.</strong> API Migration Guide</a></li><li class="chapter-item expanded affix "><li class="spacer"></li><li class="chapter-item expanded affix "><a href="misc/contributors.html">Contributors</a></li></ol>';
        // Set the current, active page, and reveal it if it's hidden
        let current_page = document.location.href.toString().split("#")[0].split("?")[0];
        if (current_page.endsWith("/")) {
            current_page += "index.html";
        }
        var links = Array.prototype.slice.call(this.querySelectorAll("a"));
        var l = links.length;
        for (var i = 0; i < l; ++i) {
            var link = links[i];
            var href = link.getAttribute("href");
            if (href && !href.startsWith("#") && !/^(?:[a-z+]+:)?\/\//.test(href)) {
                link.href = path_to_root + href;
            }
            // The "index" page is supposed to alias the first chapter in the book.
            if (link.href === current_page || (i === 0 && path_to_root === "" && current_page.endsWith("/index.html"))) {
                link.classList.add("active");
                var parent = link.parentElement;
                if (parent && parent.classList.contains("chapter-item")) {
                    parent.classList.add("expanded");
                }
                while (parent) {
                    if (parent.tagName === "LI" && parent.previousElementSibling) {
                        if (parent.previousElementSibling.classList.contains("chapter-item")) {
                            parent.previousElementSibling.classList.add("expanded");
                        }
                    }
                    parent = parent.parentElement;
                }
            }
        }
        // Track and set sidebar scroll position
        this.addEventListener('click', function(e) {
            if (e.target.tagName === 'A') {
                sessionStorage.setItem('sidebar-scroll', this.scrollTop);
            }
        }, { passive: true });
        var sidebarScrollTop = sessionStorage.getItem('sidebar-scroll');
        sessionStorage.removeItem('sidebar-scroll');
        if (sidebarScrollTop) {
            // preserve sidebar scroll position when navigating via links within sidebar
            this.scrollTop = sidebarScrollTop;
        } else {
            // scroll sidebar to current active section when navigating via "next/previous chapter" buttons
            var activeSection = document.querySelector('#sidebar .active');
            if (activeSection) {
                activeSection.scrollIntoView({ block: 'center' });
            }
        }
        // Toggle buttons
        var sidebarAnchorToggles = document.querySelectorAll('#sidebar a.toggle');
        function toggleSection(ev) {
            ev.currentTarget.parentElement.classList.toggle('expanded');
        }
        Array.from(sidebarAnchorToggles).forEach(function (el) {
            el.addEventListener('click', toggleSection);
        });
    }
}
window.customElements.define("mdbook-sidebar-scrollbox", MDBookSidebarScrollbox);
