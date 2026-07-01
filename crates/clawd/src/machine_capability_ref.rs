#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct CapabilityRef<'a> {
    namespace: &'a str,
    action: &'a str,
}

impl<'a> CapabilityRef<'a> {
    fn parse(token: &'a str) -> Option<Self> {
        let capability = token.trim().strip_prefix("capability_ref=")?;
        let (namespace, action) = capability.split_once('.')?;
        if namespace.is_empty()
            || action.is_empty()
            || !capability.bytes().all(|byte| {
                byte.is_ascii_lowercase()
                    || byte.is_ascii_digit()
                    || matches!(byte, b'_' | b'-' | b'.')
            })
        {
            return None;
        }
        Some(Self { namespace, action })
    }

    fn namespace_matches(self, namespaces: &[&str]) -> bool {
        namespaces
            .iter()
            .any(|namespace| self.namespace == namespace.trim())
    }

    fn action_has_any_segment(self, segments: &[&str]) -> bool {
        self.action
            .split(|ch| matches!(ch, '.' | '_' | '-'))
            .any(|segment| segments.iter().any(|wanted| segment == wanted.trim()))
    }

    fn action_matches(self, actions: &[&str]) -> bool {
        actions.iter().any(|action| self.action == action.trim())
    }
}

pub(crate) fn route_has_capability_namespace(
    route: &crate::RouteResult,
    namespaces: &[&str],
) -> bool {
    route_capability_refs(route).any(|capability| capability.namespace_matches(namespaces))
}

pub(crate) fn route_has_capability_action(
    route: &crate::RouteResult,
    namespaces: &[&str],
    action_segments: &[&str],
) -> bool {
    route_capability_refs(route).any(|capability| {
        capability.namespace_matches(namespaces)
            && capability.action_has_any_segment(action_segments)
    })
}

pub(crate) fn route_has_capability_action_name(
    route: &crate::RouteResult,
    namespaces: &[&str],
    actions: &[&str],
) -> bool {
    route_capability_refs(route).any(|capability| {
        capability.namespace_matches(namespaces) && capability.action_matches(actions)
    })
}

pub(crate) fn route_capability_action_for_namespaces<'a>(
    route: &'a crate::RouteResult,
    namespaces: &[&str],
) -> Option<&'a str> {
    route_capability_refs(route)
        .find(|capability| capability.namespace_matches(namespaces))
        .map(|capability| capability.action)
}

fn route_capability_refs(route: &crate::RouteResult) -> impl Iterator<Item = CapabilityRef<'_>> {
    [&route.route_reason, &route.resolved_intent]
        .into_iter()
        .flat_map(|surface| machine_context_capability_refs(surface))
}

fn machine_context_capability_refs(
    machine_context: &str,
) -> impl Iterator<Item = CapabilityRef<'_>> {
    machine_context
        .split(|ch: char| ch.is_whitespace() || matches!(ch, ';' | ',' | '(' | ')' | '[' | ']'))
        .filter_map(CapabilityRef::parse)
}
