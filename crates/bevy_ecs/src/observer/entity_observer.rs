use crate::{
    component::{Component, ComponentCloneBehavior, Mutable, StorageType},
    entity::{ComponentCloneCtx, Entity, EntityClonerBuilder, EntityMapper, SourceComponent},
    lifecycle::{ComponentHook, HookContext},
    world::World,
};
use alloc::vec::Vec;

#[cfg(feature = "bevy_reflect")]
use crate::prelude::ReflectComponent;

use super::Observer;

/// Tracks a list of entity observers for the [`Entity`] [`ObservedBy`] is added to.
#[derive(Default, Debug)]
#[cfg_attr(feature = "bevy_reflect", derive(bevy_reflect::Reflect))]
#[cfg_attr(feature = "bevy_reflect", reflect(Component, Debug))]
pub struct ObservedBy(pub(crate) Vec<Entity>);

impl ObservedBy {
    /// Provides a read-only reference to the list of entities observing this entity.
    pub fn get(&self) -> &[Entity] {
        &self.0
    }
}

impl Component for ObservedBy {
    const STORAGE_TYPE: StorageType = StorageType::SparseSet;
    type Mutability = Mutable;

    fn on_remove() -> Option<ComponentHook> {
        Some(|mut world, HookContext { entity, .. }| {
            let observed_by = {
                let mut component = world.get_mut::<ObservedBy>(entity).unwrap();
                core::mem::take(&mut component.0)
            };
            for e in observed_by {
                let (total_entities, despawned_watched_entities) = {
                    let Ok(mut entity_mut) = world.get_entity_mut(e) else {
                        continue;
                    };
                    let Some(mut state) = entity_mut.get_mut::<Observer>() else {
                        continue;
                    };
                    state.despawned_watched_entities += 1;
                    (
                        state.descriptor.entities.len(),
                        state.despawned_watched_entities as usize,
                    )
                };

                // Despawn Observer if it has no more active sources.
                if total_entities == despawned_watched_entities {
                    world.commands().entity(e).despawn();
                }
            }
        })
    }

    fn clone_behavior() -> ComponentCloneBehavior {
        ComponentCloneBehavior::Ignore
    }
}

impl EntityClonerBuilder<'_> {
    /// Sets the option to automatically add cloned entities to the observers targeting source entity.
    pub fn add_observers(&mut self, add_observers: bool) -> &mut Self {
        if add_observers {
            self.override_clone_behavior::<ObservedBy>(ComponentCloneBehavior::Custom(
                component_clone_observed_by,
            ))
        } else {
            self.remove_clone_behavior_override::<ObservedBy>()
        }
    }
}

fn component_clone_observed_by(_source: &SourceComponent, ctx: &mut ComponentCloneCtx) {
    let target = ctx.target();
    let source = ctx.source();

    ctx.queue_deferred(move |world: &mut World, _mapper: &mut dyn EntityMapper| {
        let observed_by = world
            .get::<ObservedBy>(source)
            .map(|observed_by| observed_by.0.clone())
            .expect("Source entity must have ObservedBy");

        world
            .entity_mut(target)
            .insert(ObservedBy(observed_by.clone()));

        for observer_entity in observed_by.iter().copied() {
            let mut observer_state = world
                .get_mut::<Observer>(observer_entity)
                .expect("Source observer entity must have Observer");
            observer_state.descriptor.entities.push(target);
            let event_types = observer_state.descriptor.events.clone();
            let components = observer_state.descriptor.components.clone();
            for event_type in event_types {
                let observers = world.observers.get_observers_mut(event_type);
                if components.is_empty() {
                    if let Some(map) = observers.entity_observers.get(&source).cloned() {
                        observers.entity_observers.insert(target, map);
                    }
                } else {
                    for component in &components {
                        let Some(observers) = observers.component_observers.get_mut(component)
                        else {
                            continue;
                        };
                        if let Some(map) =
                            observers.entity_component_observers.get(&source).cloned()
                        {
                            observers.entity_component_observers.insert(target, map);
                        }
                    }
                }
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use crate::{
        entity::EntityCloner,
        event::{EntityEvent, Event},
        observer::On,
        resource::Resource,
        system::ResMut,
        world::World,
    };

    #[derive(Resource, Default)]
    struct Num(usize);

    #[derive(Event, EntityEvent)]
    struct E;

    #[test]
    fn clone_entity_with_observer() {
        let mut world = World::default();
        world.init_resource::<Num>();

        let e = world
            .spawn_empty()
            .observe(|_: On<E>, mut res: ResMut<Num>| res.0 += 1)
            .id();
        world.flush();

        world.trigger_targets(E, e);

        let e_clone = world.spawn_empty().id();
        EntityCloner::build(&mut world)
            .add_observers(true)
            .clone_entity(e, e_clone);

        world.trigger_targets(E, [e, e_clone]);

        assert_eq!(world.resource::<Num>().0, 3);
    }
}
