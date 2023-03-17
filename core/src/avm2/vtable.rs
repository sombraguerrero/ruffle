use crate::avm2::activation::Activation;
use crate::avm2::method::Method;
use crate::avm2::object::{ClassObject, FunctionObject, Object};
use crate::avm2::property::{Property, PropertyClass};
use crate::avm2::property_map::PropertyMap;
use crate::avm2::scope::ScopeChain;
use crate::avm2::traits::{Trait, TraitKind};
use crate::avm2::value::Value;
use crate::avm2::Error;
use crate::avm2::Multiname;
use crate::avm2::Namespace;
use crate::avm2::QName;
use crate::string::AvmString;
use gc_arena::{Collect, GcCell, MutationContext};
use std::cell::Ref;
use std::ops::DerefMut;

#[derive(Collect, Clone, Copy)]
#[collect(no_drop)]
pub struct VTable<'gc>(GcCell<'gc, VTableData<'gc>>);

#[derive(Collect, Clone)]
#[collect(no_drop)]
pub struct VTableData<'gc> {
    /// should always be Some post-initialization
    defining_class: Option<ClassObject<'gc>>,

    /// should always be Some post-initialization
    scope: Option<ScopeChain<'gc>>,

    protected_namespace: Option<Namespace<'gc>>,

    resolved_traits: PropertyMap<'gc, Property>,

    /// Stores the `PropertyClass` for each slot,
    /// indexed by `slot_id`
    slot_classes: Vec<PropertyClass<'gc>>,

    method_table: Vec<ClassBoundMethod<'gc>>,

    default_slots: Vec<Option<Value<'gc>>>,
}

// TODO: it might make more sense to just bind the Method to the VTable (and this its class and scope) directly
// would also be nice to somehow remove the Option-ness from `defining_class` and `scope` fields for this
// to be more intuitive and cheaper
#[derive(Collect, Clone)]
#[collect(no_drop)]
pub struct ClassBoundMethod<'gc> {
    pub class: ClassObject<'gc>,
    pub scope: ScopeChain<'gc>,
    pub method: Method<'gc>,
}

impl<'gc> VTable<'gc> {
    pub fn empty(mc: MutationContext<'gc, '_>) -> Self {
        VTable(GcCell::allocate(
            mc,
            VTableData {
                defining_class: None,
                scope: None,
                protected_namespace: None,
                resolved_traits: PropertyMap::new(),
                slot_classes: vec![],
                method_table: vec![],
                default_slots: vec![],
            },
        ))
    }

    /// A special case for newcatch. A single variable (q)name that maps to slot 1.
    pub fn newcatch(mc: MutationContext<'gc, '_>, vname: &QName<'gc>) -> Self {
        let mut rt = PropertyMap::new();

        rt.insert(*vname, Property::Slot { slot_id: 1 });

        let vt = VTable(GcCell::allocate(
            mc,
            VTableData {
                defining_class: None,
                scope: None,
                protected_namespace: None,
                resolved_traits: rt,
                method_table: vec![],
                // Compilers expect `setslot 1` to work on the `newcatch` object.
                // `setslot 1` maps to index 1, so we need two slots here, because Ruffle
                // maps setslot arg directly to the slot array index, unlike AVM which does the
                // -1 shift.
                default_slots: vec![None, None],
                slot_classes: vec![PropertyClass::Any, PropertyClass::Any],
            },
        ));

        vt
    }

    pub fn duplicate(self, mc: MutationContext<'gc, '_>) -> Self {
        VTable(GcCell::allocate(mc, self.0.read().clone()))
    }

    pub fn resolved_traits(&self) -> Ref<'_, PropertyMap<'gc, Property>> {
        Ref::map(self.0.read(), |v| &v.resolved_traits)
    }

    pub fn slot_class_name(
        &self,
        slot_id: u32,
        mc: MutationContext<'gc, '_>,
    ) -> Result<Multiname<'gc>, Error<'gc>> {
        self.0
            .read()
            .slot_classes
            .get(slot_id as usize)
            .ok_or_else(|| "Invalid slot ID".into())
            .map(|c| c.get_name(mc))
    }

    pub fn get_trait(self, name: &Multiname<'gc>) -> Option<Property> {
        if name.is_attribute() {
            return None;
        }

        self.0
            .read()
            .resolved_traits
            .get_for_multiname(name)
            .cloned()
    }

    pub fn get_trait_with_ns(self, name: &Multiname<'gc>) -> Option<(Namespace<'gc>, Property)> {
        if name.is_attribute() {
            return None;
        }

        self.0
            .read()
            .resolved_traits
            .get_with_ns_for_multiname(name)
            .map(|(ns, p)| (ns, *p))
    }

    /// Coerces `value` to the type of the slot with id `slot_id`
    pub fn coerce_trait_value(
        &self,
        slot_id: u32,
        value: Value<'gc>,
        activation: &mut Activation<'_, 'gc>,
    ) -> Result<Value<'gc>, Error<'gc>> {
        // Drop the `write()` guard, as 'slot_class.coerce' may need to access this vtable.
        let mut slot_class = { self.0.read().slot_classes[slot_id as usize].clone() };

        let (value, changed) = slot_class.coerce(activation, value)?;

        // Calling coerce modified `PropertyClass` to cache the class lookup,
        // so store the new value back in the vtable.
        if changed {
            self.0.write(activation.context.gc_context).slot_classes[slot_id as usize] = slot_class;
        }
        Ok(value)
    }

    pub fn has_trait(self, name: &Multiname<'gc>) -> bool {
        self.0
            .read()
            .resolved_traits
            .get_for_multiname(name)
            .is_some()
    }

    pub fn get_method(self, disp_id: u32) -> Option<Method<'gc>> {
        self.0
            .read()
            .method_table
            .get(disp_id as usize)
            .cloned()
            .map(|x| x.method)
    }

    pub fn get_full_method(self, disp_id: u32) -> Option<ClassBoundMethod<'gc>> {
        self.0.read().method_table.get(disp_id as usize).cloned()
    }

    pub fn default_slots(&self) -> Ref<Vec<Option<Value<'gc>>>> {
        Ref::map(self.0.read(), |v| &v.default_slots)
    }

    /// Calculate the flattened list of instance traits that this class
    /// maintains.
    ///
    /// This should be run during the class finalization step, before instances
    /// are linked (as instances will further add traits to the list).
    #[allow(clippy::if_same_then_else)]
    pub fn init_vtable(
        self,
        defining_class: ClassObject<'gc>,
        traits: &[Trait<'gc>],
        scope: ScopeChain<'gc>,
        superclass_vtable: Option<Self>,
        activation: &mut Activation<'_, 'gc>,
    ) -> Result<(), Error<'gc>> {
        // Let's talk about slot_ids and disp_ids.
        // Specification is one thing, but reality is another.

        // disp_id in FP:
        // It appears that FP completely ignores it and assigns values on its own.
        // Any attempt to use `callmethod` opcode to observe the disp_id fails
        // with VerifyError.
        //
        // disp_id in Ruffle:
        // Let's just do the same. We could go the easy way and always-increment,
        // but reusing same disp_id for overriding virtual methods is a nice idea,
        // both for space savings and lets us still use call_method() internally
        // for virtual dispatch when it's safe to do so.
        // And let's error on every `callmethod` opcode and hope it never ever happens.

        // slot_id in FP:
        // It's a bit more complex here.
        //
        // If class and superclass come from the same ABC (constant pool) or superclass has no slots,
        // then slot_ids are respected; conflicts result in VerifyError.
        // You are only allowed to call `getslot` on the object if calling method,
        // callee's class and all subclasses come from the same ABC (constant pool).
        // (or class has no slots, but then `getslot` fails verification anyway as it's out-of-range)
        //
        // If class and superclass come from different ABC (constant pool) and superclass has slots,
        // then subclass's slot_ids are ignored and assigned automatically.
        // ignored, as in: even if trait's slot_id conflicts, it's not verified at all.
        //
        // In practice, this all means that compiler is allowed to use `getslot`
        // or affect/observe slots in any other way only on classes
        // it had 100% control over slot layout of, on the entire class hierarchy.
        //
        // (*in particular, trying to use `getslot` in script initializer
        //   on class defined in same script also throws VerifyError;
        //   not sure why it's treated as "different constant pool")

        // slot_id in Ruffle:
        // Currently we don't really have ability to "compare abc between
        // methods/activations/traits/etc", so let's do something simpler.
        // We try to respect slot_id whenever possible, but if a conflict arises,
        // let's just auto-assign a higher one.
        // The logic is that if we ever see a conflict, either it's a class that
        // wouldn't have passed verification in the first place, or trying to observe
        // such slot with `getslot` wouldn't have passed verification in the first place.
        // So such SWFs shouldn't be encountered in the wild.
        //
        // Worst-case is that someone can hand-craft such an SWF speficically for Ruffle
        // and be able to access private class members with `getslot/setslot,
        // so long-term it's still something we should verify.
        // (and it's far from the only verification check we lack anyway)

        let mut write = self.0.write(activation.context.gc_context);
        let write = write.deref_mut();

        write.defining_class = Some(defining_class);
        write.scope = Some(scope);

        write.protected_namespace = defining_class
            .inner_class_definition()
            .read()
            .protected_namespace();

        if let Some(superclass_vtable) = superclass_vtable {
            write.resolved_traits = superclass_vtable.0.read().resolved_traits.clone();
            write.slot_classes = superclass_vtable.0.read().slot_classes.clone();
            write.method_table = superclass_vtable.0.read().method_table.clone();
            write.default_slots = superclass_vtable.0.read().default_slots.clone();

            if let Some(protected_namespace) = write.protected_namespace {
                if let Some(super_protected_namespace) =
                    superclass_vtable.0.read().protected_namespace
                {
                    // Copy all protected traits from superclass
                    // but with this class's protected namespace
                    for (local_name, ns, prop) in superclass_vtable.0.read().resolved_traits.iter()
                    {
                        if ns == super_protected_namespace {
                            let new_name = QName::new(protected_namespace, local_name);
                            write.resolved_traits.insert(new_name, *prop);
                        }
                    }
                }
            }
        }

        let (resolved_traits, method_table, default_slots, slot_classes) = (
            &mut write.resolved_traits,
            &mut write.method_table,
            &mut write.default_slots,
            &mut write.slot_classes,
        );

        for trait_data in traits {
            match trait_data.kind() {
                TraitKind::Method { method, .. } => {
                    let entry = ClassBoundMethod {
                        class: defining_class,
                        scope,
                        method: method.clone(),
                    };
                    match resolved_traits.get(trait_data.name()) {
                        Some(Property::Method { disp_id, .. }) => {
                            let disp_id = *disp_id as usize;
                            method_table[disp_id] = entry;
                        }
                        // note: ideally overwriting other property types
                        // should be a VerifyError
                        _ => {
                            let disp_id = method_table.len() as u32;
                            method_table.push(entry);
                            resolved_traits
                                .insert(trait_data.name(), Property::new_method(disp_id));
                        }
                    }
                }
                TraitKind::Getter { method, .. } => {
                    let entry = ClassBoundMethod {
                        class: defining_class,
                        scope,
                        method: method.clone(),
                    };
                    match resolved_traits.get_mut(trait_data.name()) {
                        Some(Property::Virtual {
                            get: Some(disp_id), ..
                        }) => {
                            let disp_id = *disp_id as usize;
                            method_table[disp_id] = entry;
                        }
                        Some(Property::Virtual { get, .. }) => {
                            let disp_id = method_table.len() as u32;
                            *get = Some(disp_id);
                            method_table.push(entry);
                        }
                        _ => {
                            let disp_id = method_table.len() as u32;
                            method_table.push(entry);
                            resolved_traits
                                .insert(trait_data.name(), Property::new_getter(disp_id));
                        }
                    }
                }
                TraitKind::Setter { method, .. } => {
                    let entry = ClassBoundMethod {
                        class: defining_class,
                        scope,
                        method: method.clone(),
                    };
                    match resolved_traits.get_mut(trait_data.name()) {
                        Some(Property::Virtual {
                            set: Some(disp_id), ..
                        }) => {
                            method_table[*disp_id as usize] = entry;
                        }
                        Some(Property::Virtual { set, .. }) => {
                            let disp_id = method_table.len() as u32;
                            method_table.push(entry);
                            *set = Some(disp_id);
                        }
                        _ => {
                            let disp_id = method_table.len() as u32;
                            method_table.push(entry);
                            resolved_traits
                                .insert(trait_data.name(), Property::new_setter(disp_id));
                        }
                    }
                }
                TraitKind::Slot { slot_id, .. }
                | TraitKind::Const { slot_id, .. }
                | TraitKind::Function { slot_id, .. }
                | TraitKind::Class { slot_id, .. } => {
                    let slot_id = *slot_id;

                    let value = trait_to_default_value(scope, trait_data, activation);
                    let value = Some(value);

                    let new_slot_id = if slot_id == 0 {
                        default_slots.push(value);
                        default_slots.len() as u32 - 1
                    } else if let Some(Some(_)) = default_slots.get(slot_id as usize) {
                        // slot_id conflict
                        default_slots.push(value);
                        default_slots.len() as u32 - 1
                    } else {
                        if slot_id as usize >= default_slots.len() {
                            default_slots.resize_with(slot_id as usize + 1, Default::default);
                        }
                        default_slots[slot_id as usize] = value;
                        slot_id
                    };

                    if new_slot_id as usize >= slot_classes.len() {
                        // We will overwrite `PropertyClass::Any` when we process the slots
                        // with the ids that we just skipped over.
                        slot_classes.resize(new_slot_id as usize + 1, PropertyClass::Any);
                    }

                    let (new_prop, new_class) = match trait_data.kind() {
                        TraitKind::Slot {
                            type_name, unit, ..
                        } => (
                            Property::new_slot(new_slot_id),
                            PropertyClass::name(activation, type_name.clone(), *unit),
                        ),
                        TraitKind::Function { .. } => (
                            Property::new_slot(new_slot_id),
                            PropertyClass::Class(activation.avm2().classes().function),
                        ),
                        TraitKind::Const {
                            type_name, unit, ..
                        } => (
                            Property::new_const_slot(new_slot_id),
                            PropertyClass::name(activation, type_name.clone(), *unit),
                        ),
                        TraitKind::Class { .. } => (
                            Property::new_const_slot(new_slot_id),
                            PropertyClass::Class(activation.avm2().classes().class),
                        ),
                        _ => unreachable!(),
                    };

                    resolved_traits.insert(trait_data.name(), new_prop);
                    slot_classes[new_slot_id as usize] = new_class;
                }
            }
        }

        Ok(())
    }

    /// Retrieve a bound instance method suitable for use as a value.
    ///
    /// This returns the bound method object itself, as well as it's dispatch
    /// ID. You will need the additional properties in order to install the
    /// method into your object.
    ///
    /// You should only call this method once per reciever/name pair, and cache
    /// the result. Otherwise, code that relies on bound methods having stable
    /// object identitities (e.g. `EventDispatcher.removeEventListener`) will
    /// fail.
    pub fn make_bound_method(
        self,
        activation: &mut Activation<'_, 'gc>,
        receiver: Object<'gc>,
        disp_id: u32,
    ) -> Option<FunctionObject<'gc>> {
        if let Some(ClassBoundMethod {
            class,
            scope,
            method,
        }) = self.get_full_method(disp_id)
        {
            Some(FunctionObject::from_method(
                activation,
                method,
                scope,
                Some(receiver),
                Some(class),
            ))
        } else {
            None
        }
    }

    /// Install a const trait on the global object.
    /// This should only ever be called via `Object::install_const_late`,
    /// on the `global` object.
    pub fn install_const_trait_late(
        self,
        mc: MutationContext<'gc, '_>,
        name: QName<'gc>,
        value: Value<'gc>,
        class: ClassObject<'gc>,
    ) -> u32 {
        let mut write = self.0.write(mc);

        write.default_slots.push(Some(value));
        let new_slot_id = write.default_slots.len() as u32 - 1;
        write
            .resolved_traits
            .insert(name, Property::new_slot(new_slot_id));
        write.slot_classes.push(PropertyClass::Class(class));

        new_slot_id
    }

    /// Install an existing trait under a new name, provided by interface.
    /// This should only ever be called by `link_interfaces`.
    pub fn copy_property_for_interface(
        self,
        mc: MutationContext<'gc, '_>,
        public_name: QName<'gc>,
        interface_name: QName<'gc>,
    ) {
        let mut write = self.0.write(mc);

        let prop = write.resolved_traits.get(public_name).cloned();

        if let Some(prop) = prop {
            write.resolved_traits.insert(interface_name, prop);
        }
    }

    pub fn public_properties(self) -> Vec<(AvmString<'gc>, Property)> {
        let read = self.0.read();

        let mut props = Vec::new();

        for (name, ns, prop) in read.resolved_traits.iter() {
            if ns.is_public() {
                props.push((name, *prop));
            }
        }
        props
    }
}

fn trait_to_default_value<'gc>(
    scope: ScopeChain<'gc>,
    trait_data: &Trait<'gc>,
    activation: &mut Activation<'_, 'gc>,
) -> Value<'gc> {
    match trait_data.kind() {
        TraitKind::Slot { default_value, .. } => *default_value,
        TraitKind::Const { default_value, .. } => *default_value,
        TraitKind::Function { function, .. } => {
            FunctionObject::from_function(activation, function.clone(), scope)
                .unwrap()
                .into()
        }
        TraitKind::Class { .. } => Value::Undefined,
        _ => unreachable!(),
    }
}
