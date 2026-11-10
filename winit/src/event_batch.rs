//! A widget transition must reach the program before later queued input uses it.

pub(crate) fn dispatch<E, S>(
    events: &[E],
    mut update: impl FnMut(&[E]) -> (bool, Vec<S>),
) -> (Vec<S>, &[E]) {
    let mut statuses = Vec::with_capacity(events.len());
    for (index, event) in events.iter().enumerate() {
        let (outdated, mut reported) = update(std::slice::from_ref(event));
        statuses.append(&mut reported);
        if outdated {
            return (statuses, &events[index + 1..]);
        }
    }
    (statuses, &events[events.len()..])
}

#[cfg(test)]
mod tests {
    use super::dispatch;

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum Input {
        Page,
        Tab(u8),
        Filter,
        Enter,
    }

    #[test]
    fn a_page_message_returns_before_tabs_reach_the_old_pane() {
        let events = [Input::Page, Input::Tab(1), Input::Tab(2), Input::Enter];
        let mut delivered = vec![];
        let (statuses, pending) = dispatch(&events, |batch| {
            delivered.extend_from_slice(batch);
            (batch.contains(&Input::Page), vec![true; batch.len()])
        });
        assert_eq!(
            delivered,
            [Input::Page],
            "queued Tabs ran before the page message could be applied"
        );
        assert_eq!(statuses, [true]);
        assert_eq!(pending, &events[1..]);
    }

    #[test]
    fn rebuilding_at_each_transition_preserves_order_and_delivers_every_key_once()
     {
        let events = [
            Input::Page,
            Input::Tab(1),
            Input::Filter,
            Input::Tab(2),
            Input::Enter,
        ];
        let mut pending = events.as_slice();
        let mut pane = "Views";
        let mut history = vec![];
        let mut reported = 0;
        while !pending.is_empty() {
            let mut next = pane;
            let (statuses, rest) = dispatch(pending, |batch| {
                for event in batch {
                    history.push((pane, *event));
                    next = match event {
                        Input::Page => "Toolbar",
                        Input::Filter => "Results",
                        _ => next,
                    };
                }
                (next != pane, vec![true; batch.len()])
            });
            reported += statuses.len();
            pending = rest;
            pane = next; // Program messages apply, then the interface rebuilds.
        }
        assert_eq!(
            history,
            [
                ("Views", Input::Page),
                ("Toolbar", Input::Tab(1)),
                ("Toolbar", Input::Filter),
                ("Results", Input::Tab(2)),
                ("Results", Input::Enter)
            ]
        );
        assert_eq!(reported, events.len());
    }

    #[test]
    fn ordinary_traversal_has_no_boundary_and_empty_batches_publish_nothing() {
        let events = [Input::Tab(1), Input::Tab(2)];
        let (statuses, remaining) =
            dispatch(&events, |batch| (false, batch.to_vec()));
        assert_eq!(statuses, events);
        assert!(remaining.is_empty());
        let (statuses, remaining) =
            dispatch::<Input, Input>(&[], |batch| (false, batch.to_vec()));
        assert!(statuses.is_empty() && remaining.is_empty());
    }
}
