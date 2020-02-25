pub const ACC_PUBLIC: u16       = 0x0001;  //   X      X      X
pub const ACC_PRIVATE: u16      = 0x0002;  //   X      X      X (applicable to inner classes)
pub const ACC_PROTECTED: u16    = 0x0004;  //   X      X      X (applicable to inner classes)
pub const ACC_STATIC: u16       = 0x0008;  //   X      X      X (applicable to inner classes)
pub const ACC_FINAL: u16        = 0x0010;  //   X      X      X
pub const ACC_SYNCHRONIZED: u16 = 0x0020;  //   -      -      X  <- same value as ACC_SUPER
pub const ACC_SUPER: u16        = 0x0020;  //   X      -      -  <- same value as ACC_SYNCHRONIZED
pub const ACC_VOLATILE: u16     = 0x0040;  //   -      X      -
pub const BRIDGE: u16           = 0x0040;  //   -      -      X  <- same value as ACC_VOLATILE
pub const ACC_TRANSIENT: u16    = 0x0080;  //   -      X      -
pub const VARARGS: u16          = 0x0080;  //   -      -      X  <- same value as ACC_TRANSIENT
pub const ACC_NATIVE: u16       = 0x0100;  //   -      -      X
pub const ACC_INTERFACE: u16    = 0x0200;  //   X      -      -
pub const ACC_ABSTRACT: u16     = 0x0400;  //   X      -      X
pub const ACC_STRICT: u16       = 0x0800;  //   -      -      X
pub const ACC_SYNTHETIC: u16    = 0x1000;  //   X      X      X
pub const ACC_ANNOTATION: u16   = 0x2000;  //   X      -      -
pub const ACC_ENUM: u16         = 0x4000;  //   X      X      -

pub const APPLICABLE_TO_FIELDS: u16 =
    (ACC_PUBLIC |
    ACC_PRIVATE |
    ACC_PROTECTED |
    ACC_STATIC |
    ACC_FINAL |
    ACC_VOLATILE |
    ACC_TRANSIENT |
    ACC_SYNTHETIC |
    ACC_ENUM);

pub const APPLICABLE_TO_METHODS: u16 =
    (ACC_PUBLIC |
    ACC_PRIVATE |
    ACC_PROTECTED |
    ACC_STATIC |
    ACC_FINAL |
    ACC_SYNCHRONIZED |
    BRIDGE |
    VARARGS |
    ACC_NATIVE |
    ACC_ABSTRACT |
    ACC_STRICT |
    ACC_SYNTHETIC);

pub const APPLICABLE_TO_CLASSES: u16 =
    (ACC_PUBLIC |
    ACC_PRIVATE |
    ACC_PROTECTED |
    ACC_STATIC |
    ACC_FINAL |
    ACC_SUPER |
    ACC_INTERFACE |
    ACC_ABSTRACT |
    ACC_SYNTHETIC |
    ACC_ANNOTATION |
    ACC_ENUM);

/**
 * The modifiers that can appear in the return value of
 * {@link java.lang.Class#getModifiers()} according to
 * the Java API specification.
 */
pub const APPLICABLE_FOR_CLASS_GET_MODIFIERS: u16 =
    (ACC_PUBLIC |
    ACC_PRIVATE |
    ACC_PROTECTED |
    ACC_STATIC |
    ACC_FINAL |
    ACC_INTERFACE |
    ACC_ABSTRACT);

/* Possible states of a class description. */
/** nothing present yet */
pub const CLASS_VACANT: u8 = 0;
/** .class file contents read successfully */
pub const CLASS_LOADED: u8 = 1;
/** fields &amp; methods laid out, tib &amp; statics allocated */
pub const CLASS_RESOLVED: u8 = 2;
/** tib and jtoc populated */
pub const CLASS_INSTANTIATED: u8 = 3;
/** &lt;clinit&gt; running (allocations possible) */
pub const CLASS_INITIALIZING: u8 = 4;
/** exception occurred while running &lt;clinit&gt; class cannot be initialized successfully */
pub const CLASS_INITIALIZER_FAILED: u8 = 5;
/** statics initialized */
pub const CLASS_INITIALIZED: u8 = 6;

// Constant pool entry tags.
//
pub const TAG_UTF: u8 = 1;
pub const TAG_UNUSED: u8 = 2;
pub const TAG_INT: u8 = 3;
pub const TAG_FLOAT: u8 = 4;
pub const TAG_LONG: u8 = 5;
pub const TAG_DOUBLE: u8 = 6;
pub const TAG_TYPEREF: u8 = 7;
pub const TAG_STRING: u8 = 8;
pub const TAG_FIELDREF: u8 = 9;
pub const TAG_METHODREF: u8 = 10;
pub const TAG_INTERFACE_METHODREF: u8 = 11;
pub const TAG_MEMBERNAME_AND_DESCRIPTOR: u8 = 12;

// Type codes for class, array, and primitive types.
//
pub const CLASS_TYPE_CODE: u8 = 'L' as u8;
pub const ARRAY_TYPE_CODE: u8 = '[' as u8;
pub const VOID_TYPE_CODE: u8 = 'V' as u8;
pub const BOOLEAN_TYPE_CODE: u8 = 'Z' as u8;
pub const BYTE_TYPE_CODE: u8 = 'B' as u8;
pub const SHORT_TYPE_CODE: u8 = 'S' as u8;
pub const INT_TYPE_CODE: u8 = 'I' as u8;
pub const LONG_TYPE_CODE: u8 = 'J' as u8;
pub const FLOAT_TYPE_CODE: u8 = 'F' as u8;
pub const DOUBLE_TYPE_CODE: u8 = 'D' as u8;
pub const CHAR_TYPE_CODE: u8 = 'C' as u8;

// Constants for our internal encoding of constant pools.
/** Constant pool entry for a UTF-8 encoded atom */
pub const CP_UTF: u8 = 0;
/** Constant pool entry for int literal */
pub const CP_INT: u8 = 1;
/** Constant pool entry for long literal */
pub const CP_LONG: u8 = 2;
/** Constant pool entry for float literal */
pub const CP_FLOAT: u8 = 3;
/** Constant pool entry for double literal */
pub const CP_DOUBLE: u8 = 4;
/** Constant pool entry for string literal (for annotations, may be other objects) */
pub const CP_STRING: u8 = 5;
/** Constant pool entry for member (field or method) reference */
pub const CP_MEMBER: u8 = 6;
/** Constant pool entry for type reference or class literal */
pub const CP_CLASS: u8 = 7;