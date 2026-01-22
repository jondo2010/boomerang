# Glossary

- Action: An event source within a reactor. Can be logical or physical.
- Determinism: Given the same input events and tags, the runtime produces the same outputs and state transitions.
- Input port: A typed input declared with `#[input]` that can trigger reactions.
- Output port: A typed output declared with `#[output]` that reactions can write to.
- Physical action: An action that receives nondeterministic data from outside the runtime.
- Reaction: A function that runs when its triggers are present at a tag.
- Reactor: A component defined with `#[reactor]` that encapsulates state, actions, and reactions.
- Tag: The pair of logical time and microstep that orders events.
- Timer: A scheduled action that fires at a logical time offset and period.
