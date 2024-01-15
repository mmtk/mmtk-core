# Development Principles

These define how we deal with code.

## Why we need principles?
* They keep the project focused on the right direction.
* They keep the project disciplined.
* They make users' expectation for the project clear.

## Principles

* **Enforcability**
  * Meta-principle. We enforce the principles in a systematic way. We use automated checks to enforce the principles as much as we possible.
* **Performance**
  * Our goal is to build high performance memory management systems.
  * We need to have the tools (for measuring performance) that allow us to create high performance systems.
* **Flexibility**
  * Flexibility is a prerequisite to creative, ambitious engineering, and is thus a key principle.
  * The design should maintain clear abstraction.
  * Flexibility should not be at odds with performance.   Our motto (from Ken Kennedy): _abstraction without guilt_.
  * Encapsulation is our key weapon in ensuring flexibility.
* **Clarity/Accessibility**
  * Our goal is to provide a toolkit that is widely used.    Consequently:
    * The code should be clear and easy to understand.
    * A good programmer (with no GC expertise) should be able to understand the code and make changes.
    * Use standard coding styles, use standard license, support standard IDEs, etc.
    * Systems need to work 'out of the box'.
* **Security**
  * Our system must be secure.
  * We should be a platform for research into memory management security.
