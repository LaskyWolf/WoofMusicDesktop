use trop::audio::player_controller::player_controller::{effects_graph, filter_chain};

const REVERB: &str = "[verb]volume@verbsend=0,aecho=0.7:0.7:13|19|29|37:0.5|0.4|0.3|0.2,aecho=0:0.6:41|67|97|131|173|223|281|353|439|547|677|839:0.6|0.5|0.42|0.35|0.29|0.24|0.2|0.17|0.14|0.12|0.1|0.08[wet]";

#[test]
fn effects_graph_matches_swift() {
    assert_eq!(
        effects_graph(None),
        format!(
            "asplit=2[dry][verb];{REVERB};[dry]volume@dry=1[d];[d][wet]amix=inputs=2:normalize=0:duration=first"
        )
    );
    assert_eq!(
        effects_graph(Some(0.5)),
        format!(
            "asplit=3[dry][echo][verb];[echo]volume@echosend=0,aecho=0:1:500.0000|1000.0000|1500.0000|2000.0000:0.5000|0.3000|0.1800|0.1080[rep];{REVERB};[dry]volume@dry=1[d];[d][rep][wet]amix=inputs=3:normalize=0:duration=first"
        )
    );
}

#[test]
fn filter_chain_labels_each_automix_filter() {
    let graph = effects_graph(None);
    assert_eq!(
        filter_chain(Some(-24.0), None, true, true),
        format!(
            "@automixbass:bass=f=180:g=-24.0000,@automixhp:highpass=f=1000:m=0,@automixlp:lowpass=f=1000:m=0,@automixfx:lavfi=graph=%{}%{graph}",
            graph.len()
        )
    );
    assert_eq!(filter_chain(None, None, false, false), "");
}
