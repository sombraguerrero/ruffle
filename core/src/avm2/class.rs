//! AVM2 classes

use crate::avm2::activation::Activation;
use crate::avm2::method::{Method, NativeMethodImpl};
use crate::avm2::object::{ClassObject, Object};
use crate::avm2::script::TranslationUnit;
use crate::avm2::traits::{Trait, TraitKind};
use crate::avm2::value::Value;
use crate::avm2::Error;
use crate::avm2::Multiname;
use crate::avm2::Namespace;
use crate::avm2::QName;
use bitflags::bitflags;
use gc_arena::{Collect, GcCell, MutationContext};
use std::fmt;
use std::hash::{Hash, Hasher};
use std::ops::Deref;
use swf::avm2::types::{
    Class as AbcClass, Instance as AbcInstance, Method as AbcMethod, MethodBody as AbcMethodBody,
};

use super::method::ParamConfig;
use super::string::AvmString;

bitflags! {
    /// All possible attributes for a given class.
    #[derive(Clone, Copy)]
    pub struct ClassAttributes: u8 {
        /// Class is sealed, attempts to set or init dynamic properties on an
        /// object will generate a runtime error.
        const SEALED    = 1 << 0;

        /// Class is final, attempts to construct child classes from it will
        /// generate a verification error.
        const FINAL     = 1 << 1;

        /// Class is an interface.
        const INTERFACE = 1 << 2;

        /// Class accepts type parameters.
        const GENERIC = 1 << 3;
    }
}

/// A function that can be used to allocate instances of a class.
///
/// By default, the `implicit_allocator` is used, which attempts to use the base
/// class's allocator, and defaults to `ScriptObject` otherwise. Custom
/// allocators anywhere in the class inheritance chain can change the
/// representation of all subclasses that use the implicit allocator.
///
/// Parameters for the allocator are:
///
///  * `class` - The class object that is being allocated. This must be the
///  current class (using a superclass will cause the wrong class to be
///  read for traits).
///  * `activation` - The current AVM2 activation.
pub type AllocatorFn =
    for<'gc> fn(ClassObject<'gc>, &mut Activation<'_, 'gc>) -> Result<Object<'gc>, Error<'gc>>;

#[derive(Clone, Collect)]
#[collect(require_static)]
pub struct Allocator(pub AllocatorFn);

impl fmt::Debug for Allocator {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("Allocator")
            .field(&"<native code>".to_string())
            .finish()
    }
}

/// A loaded ABC Class which can be used to construct objects with.
#[derive(Clone, Collect)]
#[collect(no_drop)]
pub struct Class<'gc> {
    /// The name of the class.
    name: QName<'gc>,

    /// The type parameters for this class.
    params: Vec<GcCell<'gc, Class<'gc>>>,

    /// The name of this class's superclass.
    super_class: Option<Multiname<'gc>>,

    /// Attributes of the given class.
    #[collect(require_static)]
    attributes: ClassAttributes,

    /// The namespace that protected traits of this class are stored into.
    protected_namespace: Option<Namespace<'gc>>,

    /// The list of interfaces this class directly implements. This does not include any
    /// superinterfaces, nor interfaces implemented by the superclass.
    direct_interfaces: Vec<Multiname<'gc>>,

    /// The instance allocator for this class.
    ///
    /// If `None`, then instances of this object will be allocated the same way
    /// as the superclass specifies; or if there is no superclass, it will be
    /// allocated as a `ScriptObject`.
    instance_allocator: Option<Allocator>,

    /// The instance initializer for this class.
    ///
    /// Must be called each time a new class instance is constructed.
    instance_init: Method<'gc>,

    /// The native instance initializer for this class.
    ///
    /// This may be provided to allow natively-constructed classes to
    /// initialize themselves in a different manner from user-constructed ones.
    /// For example, the user-accessible constructor may error out (as it's not
    /// a valid class to construct for users), but native code may still call
    /// it's constructor stack.
    ///
    /// By default, a class's `native_instance_init` will be initialized to the
    /// same method as the regular one. You must specify a separate native
    /// initializer to change initialization behavior based on what code is
    /// constructing the class.
    native_instance_init: Method<'gc>,

    /// Instance traits for a given class.
    ///
    /// These are accessed as normal instance properties; they should not be
    /// present on prototypes, but instead should shadow any prototype
    /// properties that would match.
    instance_traits: Vec<Trait<'gc>>,

    /// The class initializer for this class.
    ///
    /// Must be called once and only once prior to any use of this class.
    class_init: Method<'gc>,

    /// Whether or not the class initializer has already been called.
    class_initializer_called: bool,

    /// The customization point for `Class(args...)` without `new`
    /// If None, a simple coercion is done.
    call_handler: Option<Method<'gc>>,

    /// The class initializer for specializations of this class.
    ///
    /// Only applies for generic classes. Must be called once and only once
    /// per specialization, prior to any use of the specialized class.
    specialized_class_init: Method<'gc>,

    /// Static traits for a given class.
    ///
    /// These are accessed as class object properties.
    class_traits: Vec<Trait<'gc>>,

    /// Whether or not this `Class` has loaded its traits or not.
    traits_loaded: bool,

    /// Whether or not this is a system-defined class.
    ///
    /// System defined classes are allowed to have illegal trait configurations
    /// without throwing a VerifyError.
    is_system: bool,
}

impl<'gc> Class<'gc> {
    /// Create a new class.
    ///
    /// This function is primarily intended for use by native code to define
    /// builtin classes. The absolute minimum necessary to define a class is
    /// required here; further methods allow further changes to the class.
    ///
    /// Classes created in this way cannot have traits loaded from an ABC file
    /// using `load_traits`.
    pub fn new(
        name: QName<'gc>,
        super_class: Option<Multiname<'gc>>,
        instance_init: Method<'gc>,
        class_init: Method<'gc>,
        mc: MutationContext<'gc, '_>,
    ) -> GcCell<'gc, Self> {
        let native_instance_init = instance_init.clone();

        GcCell::allocate(
            mc,
            Self {
                name,
                params: Vec::new(),
                super_class,
                attributes: ClassAttributes::empty(),
                protected_namespace: None,
                direct_interfaces: Vec::new(),
                instance_allocator: None,
                instance_init,
                native_instance_init,
                instance_traits: Vec::new(),
                class_init,
                class_initializer_called: false,
                call_handler: None,
                class_traits: Vec::new(),
                specialized_class_init: Method::from_builtin(
                    |_, _, _| Ok(Value::Undefined),
                    "<Null specialization constructor>",
                    mc,
                ),
                traits_loaded: true,
                is_system: true,
            },
        )
    }

    /// Apply type parameters to an existing class.
    ///
    /// This is used to parameterize a generic type. The returned class will no
    /// longer be generic.
    pub fn with_type_params(
        &self,
        params: &[GcCell<'gc, Class<'gc>>],
        mc: MutationContext<'gc, '_>,
    ) -> GcCell<'gc, Class<'gc>> {
        let mut new_class = self.clone();

        new_class.params = params.to_vec();
        new_class.attributes.remove(ClassAttributes::GENERIC);
        new_class.class_init = new_class.specialized_class_init.clone();
        new_class.class_initializer_called = false;

        if params.len() > 1 {
            panic!(
                "More than one type parameter is unsupported: {:?}",
                self.name()
            );
        }

        // FIXME - we should store a `Multiname` instead of a `QName`, and use the
        // `params` field. For now, this is good enough to get tests passing
        let name_with_params = format!(
            "{}.<{}>",
            new_class.name.local_name(),
            params[0].read().name().to_qualified_name(mc)
        );

        new_class.name = QName::new(
            new_class.name.namespace(),
            AvmString::new_utf8(mc, name_with_params),
        );

        GcCell::allocate(mc, new_class)
    }

    /// Set the attributes of the class (sealed/final/interface status).
    pub fn set_attributes(&mut self, attributes: ClassAttributes) {
        self.attributes = attributes;
    }

    /// Construct a class from a `TranslationUnit` and its class index.
    ///
    /// The returned class will be allocated, but no traits will be loaded. The
    /// caller is responsible for storing the class in the `TranslationUnit`
    /// and calling `load_traits` to complete the trait-loading process.
    pub fn from_abc_index(
        unit: TranslationUnit<'gc>,
        class_index: u32,
        activation: &mut Activation<'_, 'gc>,
    ) -> Result<GcCell<'gc, Self>, Error<'gc>> {
        let abc = unit.abc();
        let abc_class: Result<&AbcClass, Error<'gc>> = abc
            .classes
            .get(class_index as usize)
            .ok_or_else(|| "LoadError: Class index not valid".into());
        let abc_class = abc_class?;

        let abc_instance: Result<&AbcInstance, Error<'gc>> = abc
            .instances
            .get(class_index as usize)
            .ok_or_else(|| "LoadError: Instance index not valid".into());
        let abc_instance = abc_instance?;

        let name =
            QName::from_abc_multiname(unit, abc_instance.name, activation.context.gc_context)?;
        let super_class = if abc_instance.super_name.0 == 0 {
            None
        } else {
            Some(
                unit.pool_multiname_static(abc_instance.super_name, activation.context.gc_context)?
                    .deref()
                    .clone(),
            )
        };

        let protected_namespace = if let Some(ns) = &abc_instance.protected_namespace {
            Some(unit.pool_namespace(*ns, activation.context.gc_context)?)
        } else {
            None
        };

        let mut interfaces = Vec::with_capacity(abc_instance.interfaces.len());
        for interface_name in &abc_instance.interfaces {
            interfaces.push(
                unit.pool_multiname_static(*interface_name, activation.context.gc_context)?
                    .deref()
                    .clone(),
            );
        }

        let instance_init = unit.load_method(abc_instance.init_method, false, activation)?;
        let mut native_instance_init = instance_init.clone();
        let class_init = unit.load_method(abc_class.init_method, false, activation)?;
        let mut native_call_handler = None;

        let mut attributes = ClassAttributes::empty();
        attributes.set(ClassAttributes::SEALED, abc_instance.is_sealed);
        attributes.set(ClassAttributes::FINAL, abc_instance.is_final);
        attributes.set(ClassAttributes::INTERFACE, abc_instance.is_interface);

        let mut instance_allocator = None;

        // When loading a class from our playerglobal, grab the corresponding native
        // allocator function from the table (which may be `None`)
        if unit.domain().is_avm2_global_domain(activation) {
            instance_allocator = activation.avm2().native_instance_allocator_table
                [class_index as usize]
                .map(|(_name, ptr)| Allocator(ptr));

            if let Some((name, table_native_init)) =
                activation.avm2().native_instance_init_table[class_index as usize]
            {
                let method = Method::from_builtin_and_params(
                    table_native_init,
                    name,
                    instance_init.signature().to_vec(),
                    instance_init.is_variadic(),
                    activation.context.gc_context,
                );
                native_instance_init = method;
            }

            if let Some((name, table_native_call_handler)) =
                activation.avm2().native_call_handler_table[class_index as usize]
            {
                let method = Method::from_builtin_and_params(
                    table_native_call_handler,
                    name,
                    vec![ParamConfig::of_type(
                        "val",
                        Multiname::any(activation.context.gc_context),
                    )],
                    false,
                    activation.context.gc_context,
                );
                native_call_handler = Some(method);
            }
        }

        Ok(GcCell::allocate(
            activation.context.gc_context,
            Self {
                name,
                params: Vec::new(),
                super_class,
                attributes,
                protected_namespace,
                direct_interfaces: interfaces,
                instance_allocator,
                instance_init,
                native_instance_init,
                instance_traits: Vec::new(),
                class_init,
                class_initializer_called: false,
                call_handler: native_call_handler,
                class_traits: Vec::new(),
                specialized_class_init: Method::from_builtin(
                    |_, _, _| Ok(Value::Undefined),
                    "<Null specialization constructor>",
                    activation.context.gc_context,
                ),
                traits_loaded: false,
                is_system: false,
            },
        ))
    }

    /// Finish the class-loading process by loading traits.
    ///
    /// This process must be done after the `Class` has been stored in the
    /// `TranslationUnit`. Failing to do so runs the risk of runaway recursion
    /// or double-borrows. It should be done before the class is actually
    /// instantiated into an `Object`.
    pub fn load_traits(
        &mut self,
        unit: TranslationUnit<'gc>,
        class_index: u32,
        activation: &mut Activation<'_, 'gc>,
    ) -> Result<(), Error<'gc>> {
        if self.traits_loaded {
            return Ok(());
        }

        self.traits_loaded = true;

        let abc = unit.abc();
        let abc_class: Result<&AbcClass, Error<'gc>> = abc
            .classes
            .get(class_index as usize)
            .ok_or_else(|| "LoadError: Class index not valid".into());
        let abc_class = abc_class?;

        let abc_instance: Result<&AbcInstance, Error<'gc>> = abc
            .instances
            .get(class_index as usize)
            .ok_or_else(|| "LoadError: Instance index not valid".into());
        let abc_instance = abc_instance?;

        for abc_trait in abc_instance.traits.iter() {
            self.instance_traits
                .push(Trait::from_abc_trait(unit, abc_trait, activation)?);
        }

        for abc_trait in abc_class.traits.iter() {
            self.class_traits
                .push(Trait::from_abc_trait(unit, abc_trait, activation)?);
        }

        Ok(())
    }

    /// Completely validate a class against it's resolved superclass.
    ///
    /// This should be called at class creation time once the superclass name
    /// has been resolved. It will return Ok for a valid class, and a
    /// VerifyError for any invalid class.
    pub fn validate_class(&self, superclass: Option<ClassObject<'gc>>) -> Result<(), Error<'gc>> {
        // System classes do not throw verify errors.
        if self.is_system {
            return Ok(());
        }

        if let Some(superclass) = superclass {
            for instance_trait in self.instance_traits.iter() {
                let is_protected =
                    self.protected_namespace() == Some(instance_trait.name().namespace());

                let mut current_superclass = Some(superclass);
                let mut did_override = false;

                while let Some(superclass) = current_superclass {
                    let superclass_def = superclass.inner_class_definition();
                    let read = superclass_def.read();

                    for supertrait in read.instance_traits.iter() {
                        let super_name = supertrait.name();
                        let my_name = instance_trait.name();

                        let names_match = super_name.local_name() == my_name.local_name()
                            && (super_name.namespace() == my_name.namespace()
                                || (is_protected
                                    && read.protected_namespace() == Some(super_name.namespace())));
                        if names_match {
                            match (supertrait.kind(), instance_trait.kind()) {
                                //Getter/setter pairs do NOT override one another
                                (TraitKind::Getter { .. }, TraitKind::Setter { .. }) => continue,
                                (TraitKind::Setter { .. }, TraitKind::Getter { .. }) => continue,
                                (_, _) => did_override = true,
                            }

                            if supertrait.is_final() {
                                return Err(format!("VerifyError: Trait {} in class {} overrides final trait {} in class {}", instance_trait.name().local_name(), self.name().local_name(), supertrait.name().local_name(), read.name().local_name()).into());
                            }

                            if !instance_trait.is_override() {
                                return Err(format!("VerifyError: Trait {} in class {} has same name as trait {} in class {}, but does not override it", instance_trait.name().local_name(), self.name().local_name(), supertrait.name().local_name(), read.name().local_name()).into());
                            }

                            break;
                        }
                    }

                    // The superclass is already validated so we don't need to
                    // check further.
                    if did_override {
                        break;
                    }

                    current_superclass = superclass.superclass_object();
                }

                if instance_trait.is_override() && !did_override {
                    return Err(format!("VerifyError: Trait {} in class {} marked as override, does not override any other trait", instance_trait.name().local_name(), self.name().local_name()).into());
                }
            }
        }

        Ok(())
    }

    pub fn for_activation(
        activation: &mut Activation<'_, 'gc>,
        translation_unit: TranslationUnit<'gc>,
        method: &AbcMethod,
        body: &AbcMethodBody,
    ) -> Result<GcCell<'gc, Self>, Error<'gc>> {
        let name =
            translation_unit.pool_string(method.name.as_u30(), activation.context.gc_context)?;
        let mut traits = Vec::with_capacity(body.traits.len());

        for trait_entry in body.traits.iter() {
            traits.push(Trait::from_abc_trait(
                translation_unit,
                trait_entry,
                activation,
            )?);
        }

        Ok(GcCell::allocate(
            activation.context.gc_context,
            Self {
                name: QName::new(activation.avm2().public_namespace, name),
                params: Vec::new(),
                super_class: None,
                attributes: ClassAttributes::empty(),
                protected_namespace: None,
                direct_interfaces: Vec::new(),
                instance_allocator: None,
                instance_init: Method::from_builtin(
                    |_, _, _| Ok(Value::Undefined),
                    "<Activation object constructor>",
                    activation.context.gc_context,
                ),
                native_instance_init: Method::from_builtin(
                    |_, _, _| Ok(Value::Undefined),
                    "<Activation object constructor>",
                    activation.context.gc_context,
                ),
                instance_traits: traits,
                class_init: Method::from_builtin(
                    |_, _, _| Ok(Value::Undefined),
                    "<Activation object class constructor>",
                    activation.context.gc_context,
                ),
                specialized_class_init: Method::from_builtin(
                    |_, _, _| Ok(Value::Undefined),
                    "<Activation object specialization constructor>",
                    activation.context.gc_context,
                ),
                class_initializer_called: false,
                call_handler: None,
                class_traits: Vec::new(),
                traits_loaded: true,
                is_system: false,
            },
        ))
    }

    pub fn name(&self) -> QName<'gc> {
        self.name
    }

    pub fn set_name(&mut self, name: QName<'gc>) {
        self.name = name;
    }

    pub fn super_class_name(&self) -> &Option<Multiname<'gc>> {
        &self.super_class
    }

    pub fn protected_namespace(&self) -> Option<Namespace<'gc>> {
        self.protected_namespace
    }

    #[inline(never)]
    pub fn define_constant_number_class_traits(
        &mut self,
        namespace: Namespace<'gc>,
        items: &[(&'static str, f64)],
        activation: &mut Activation<'_, 'gc>,
    ) {
        for &(name, value) in items {
            self.define_class_trait(Trait::from_const(
                QName::new(namespace, name),
                Multiname::new(activation.avm2().public_namespace, "Number"),
                Some(value.into()),
            ));
        }
    }
    #[inline(never)]
    pub fn define_constant_uint_class_traits(
        &mut self,
        namespace: Namespace<'gc>,
        items: &[(&'static str, u32)],
        activation: &mut Activation<'_, 'gc>,
    ) {
        for &(name, value) in items {
            self.define_class_trait(Trait::from_const(
                QName::new(namespace, name),
                Multiname::new(activation.avm2().public_namespace, "uint"),
                Some(value.into()),
            ));
        }
    }
    #[inline(never)]
    pub fn define_constant_int_class_traits(
        &mut self,
        namespace: Namespace<'gc>,
        items: &[(&'static str, i32)],
        activation: &mut Activation<'_, 'gc>,
    ) {
        for &(name, value) in items {
            self.define_class_trait(Trait::from_const(
                QName::new(namespace, name),
                Multiname::new(activation.avm2().public_namespace, "int"),
                Some(value.into()),
            ));
        }
    }
    #[inline(never)]
    pub fn define_builtin_class_properties(
        &mut self,
        mc: MutationContext<'gc, '_>,
        namespace: Namespace<'gc>,
        items: &[(
            &'static str,
            Option<NativeMethodImpl>,
            Option<NativeMethodImpl>,
        )],
    ) {
        for &(name, getter, setter) in items {
            if let Some(getter) = getter {
                self.define_class_trait(Trait::from_getter(
                    QName::new(namespace, name),
                    Method::from_builtin(getter, name, mc),
                ));
            }
            if let Some(setter) = setter {
                self.define_class_trait(Trait::from_setter(
                    QName::new(namespace, name),
                    Method::from_builtin(setter, name, mc),
                ));
            }
        }
    }
    #[inline(never)]
    pub fn define_builtin_instance_methods(
        &mut self,
        mc: MutationContext<'gc, '_>,
        namespace: Namespace<'gc>,
        items: &[(&'static str, NativeMethodImpl)],
    ) {
        for &(name, value) in items {
            self.define_instance_trait(Trait::from_method(
                QName::new(namespace, name),
                Method::from_builtin(value, name, mc),
            ));
        }
    }
    #[inline(never)]
    pub fn define_builtin_class_methods(
        &mut self,
        mc: MutationContext<'gc, '_>,
        namespace: Namespace<'gc>,
        items: &[(&'static str, NativeMethodImpl)],
    ) {
        for &(name, value) in items {
            self.define_class_trait(Trait::from_method(
                QName::new(namespace, name),
                Method::from_builtin(value, name, mc),
            ));
        }
    }
    #[inline(never)]
    pub fn define_builtin_instance_properties(
        &mut self,
        mc: MutationContext<'gc, '_>,
        namespace: Namespace<'gc>,
        items: &[(
            &'static str,
            Option<NativeMethodImpl>,
            Option<NativeMethodImpl>,
        )],
    ) {
        for &(name, getter, setter) in items {
            if let Some(getter) = getter {
                self.define_instance_trait(Trait::from_getter(
                    QName::new(namespace, name),
                    Method::from_builtin(getter, name, mc),
                ));
            }
            if let Some(setter) = setter {
                self.define_instance_trait(Trait::from_setter(
                    QName::new(namespace, name),
                    Method::from_builtin(setter, name, mc),
                ));
            }
        }
    }
    #[inline(never)]
    pub fn define_slot_number_instance_traits(
        &mut self,
        namespace: Namespace<'gc>,
        items: &[(&'static str, Option<f64>)],
        activation: &mut Activation<'_, 'gc>,
    ) {
        for &(name, value) in items {
            self.define_instance_trait(Trait::from_slot(
                QName::new(namespace, name),
                Multiname::new(activation.avm2().public_namespace, "Number"),
                value.map(|v| v.into()),
            ));
        }
    }

    /// Define a trait on the class.
    ///
    /// Class traits will be accessible as properties on the class object.
    pub fn define_class_trait(&mut self, my_trait: Trait<'gc>) {
        self.class_traits.push(my_trait);
    }

    /// Return class traits provided by this class.
    pub fn class_traits(&self) -> &[Trait<'gc>] {
        &self.class_traits[..]
    }

    /// Define a trait on instances of the class.
    ///
    /// Instance traits will be accessible as properties on instances of the
    /// class. They will not be accessible on the class prototype, and any
    /// properties defined on the prototype will be shadowed by these traits.
    pub fn define_instance_trait(&mut self, my_trait: Trait<'gc>) {
        self.instance_traits.push(my_trait);
    }

    /// Return instance traits provided by this class.
    pub fn instance_traits(&self) -> &[Trait<'gc>] {
        &self.instance_traits[..]
    }

    /// Get this class's instance allocator.
    ///
    /// If `None`, then you should use the instance allocator of the superclass
    /// or allocate as a `ScriptObject` if no such class exists.
    pub fn instance_allocator(&self) -> Option<AllocatorFn> {
        self.instance_allocator.as_ref().map(|a| a.0)
    }

    /// Set this class's instance allocator.
    pub fn set_instance_allocator(&mut self, alloc: AllocatorFn) {
        self.instance_allocator = Some(Allocator(alloc));
    }

    /// Get this class's instance initializer.
    pub fn instance_init(&self) -> Method<'gc> {
        self.instance_init.clone()
    }

    /// Get this class's native-code instance initializer.
    pub fn native_instance_init(&self) -> Method<'gc> {
        self.native_instance_init.clone()
    }

    /// Set a native-code instance initializer for this class.
    pub fn set_native_instance_init(&mut self, new_native_init: Method<'gc>) {
        self.native_instance_init = new_native_init;
    }

    /// Get this class's class initializer.
    pub fn class_init(&self) -> Method<'gc> {
        self.class_init.clone()
    }

    /// Set a call handler for this class.
    pub fn set_call_handler(&mut self, new_call_handler: Method<'gc>) {
        self.call_handler = Some(new_call_handler);
    }

    /// Get this class's call handler.
    pub fn call_handler(&self) -> Option<Method<'gc>> {
        self.call_handler.clone()
    }

    /// Check if the class has already been initialized.
    pub fn is_class_initialized(&self) -> bool {
        self.class_initializer_called
    }

    /// Mark the class as initialized.
    pub fn mark_class_initialized(&mut self) {
        self.class_initializer_called = true;
    }

    /// Set the class initializer for specializations of this class.
    pub fn set_specialized_init(&mut self, specialized_init: Method<'gc>) {
        self.specialized_class_init = specialized_init;
    }

    pub fn direct_interfaces(&self) -> &[Multiname<'gc>] {
        &self.direct_interfaces
    }

    pub fn implements(&mut self, iface: Multiname<'gc>) {
        self.direct_interfaces.push(iface)
    }

    /// Determine if this class is sealed (no dynamic properties)
    pub fn is_sealed(&self) -> bool {
        self.attributes.contains(ClassAttributes::SEALED)
    }

    /// Determine if this class is final (cannot be subclassed)
    pub fn is_final(&self) -> bool {
        self.attributes.contains(ClassAttributes::FINAL)
    }

    /// Determine if this class is an interface
    pub fn is_interface(&self) -> bool {
        self.attributes.contains(ClassAttributes::INTERFACE)
    }

    /// Determine if this class is generic (can be specialized)
    pub fn is_generic(&self) -> bool {
        self.attributes.contains(ClassAttributes::GENERIC)
    }

    pub fn params(&self) -> &[GcCell<'gc, Class<'gc>>] {
        &self.params[..]
    }
}

pub struct ClassHashWrapper<'gc>(pub GcCell<'gc, Class<'gc>>);

impl<'gc> PartialEq for ClassHashWrapper<'gc> {
    fn eq(&self, other: &Self) -> bool {
        GcCell::ptr_eq(self.0, other.0)
    }
}
impl<'gc> Eq for ClassHashWrapper<'gc> {}

impl<'gc> Hash for ClassHashWrapper<'gc> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.0.as_ptr().hash(state);
    }
}
