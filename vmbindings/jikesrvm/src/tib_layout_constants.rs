use mmtk::SelectedConstraints::*;

/** Number of slots reserved for interface method pointers. */
pub const IMT_METHOD_SLOTS: usize = 0; //VM.BuildForIMTInterfaceInvocation ? 29 : 0;

/** First slot of TIB points to RVMType (slot 0 in above diagram). */
pub const TIB_TYPE_INDEX: usize = 0;

/** A vector of ids for classes that this one extends. See
 DynamicTypeCheck.java */
pub const TIB_SUPERCLASS_IDS_INDEX: usize = TIB_TYPE_INDEX + 1;

/** Does this class implement the ith interface? See DynamicTypeCheck.java */
pub const TIB_DOES_IMPLEMENT_INDEX: usize = TIB_SUPERCLASS_IDS_INDEX + 1;

/** The TIB of the elements type of an array (may be {@code null} in fringe cases
 *  when element type couldn't be resolved during array resolution).
 *  Will be {@code null} when not an array.
 */
pub const TIB_ARRAY_ELEMENT_TIB_INDEX: usize = TIB_DOES_IMPLEMENT_INDEX + 1;

/**
 * A pointer to either an ITable or InterfaceMethodTable (IMT)
 * depending on which dispatch implementation we are using.
 */
pub const TIB_INTERFACE_DISPATCH_TABLE_INDEX: usize = TIB_ARRAY_ELEMENT_TIB_INDEX + 1;

/**
 *  A set of 0 or more specialized methods used in the VM such as for GC scanning.
 */
pub const TIB_FIRST_SPECIALIZED_METHOD_INDEX: usize = TIB_INTERFACE_DISPATCH_TABLE_INDEX + 1;

/**
 * Next group of slots point to virtual method code blocks (slots V1..VN in above diagram).
 */
pub const TIB_FIRST_VIRTUAL_METHOD_INDEX: usize = TIB_FIRST_SPECIALIZED_METHOD_INDEX
    + NUM_SPECIALIZED_SCANS; // + SpecializedMethodManager.numSpecializedMethods();

/**
 * Special value returned by RVMClassLoader.getFieldOffset() or
 * RVMClassLoader.getMethodOffset() to indicate fields or methods
 * that must be accessed via dynamic linking code because their
 * offset is not yet known or the class's static initializer has not
 * yet been run.
 *
 *  We choose a value that will never match a valid jtoc-,
 *  instance-, or virtual method table- offset. Short.MIN_VALUE+1 is
 *  a good value:
 *
 *  <ul>
 *  <li>the jtoc offsets are aligned and this value should be
 *  too huge to address the table</li>
 *  <li>instance field offsets are always &gt;= -4 (TODO check if this is still correct)</li>
 *  <li>virtual method offsets are always positive w.r.t. TIB pointer</li>
 *  <li>fits into a PowerPC 16bit immediate operand</li>
 *   </ul>
 */
pub const NEEDS_DYNAMIC_LINK: isize = i16::min_value() as isize + 1;