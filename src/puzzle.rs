use super::Board6;
use lazy_static::lazy_static;
use std::fs::read_to_string;
use topaz_tak::{generate_all_moves, search::proof::TinueSearch, GameMove, Position, TakBoard};

pub fn new_puzzle(id: usize) -> Option<PuzzleState> {
    // Todo make sure id is valid
    Some(PuzzleState::new(id, PUZZLES.get(id)?))
}

pub fn puzzle_length() -> usize {
    PUZZLES.len()
}

lazy_static! {
    static ref PUZZLES: Vec<PuzzleData> = {
        let data = read_to_string("tinue_data.csv").unwrap();
        data.lines()
            .skip(1)
            .map(|line| {
                let split: Vec<_> = line.split(";").collect();
                let id = split[0];
                let tps = split[1].to_string();
                let pv = split[2];
                let pv = pv.split(" ").map(|x| x.to_string()).collect::<Vec<_>>();
                let nodes = split[3].parse::<f32>().unwrap();
                let root_nodes = split[4].parse::<f32>().unwrap();
                let mut difficulty = 0.0;
                if root_nodes >= 4.0 {
                    difficulty += 0.5;
                }
                difficulty += (nodes / 50_000.0).clamp(0.0, 1.5);
                difficulty += (pv.len() as f32 / 5.0).clamp(0.0, 2.0);
                PuzzleData {
                    game_id: id.parse().unwrap(),
                    tps,
                    pv,
                    difficulty,
                }
            })
            .collect()
    };
}

pub struct PuzzleState {
    puzzle_num: usize,
    original_game: usize,
    active_moves: Vec<String>,
    active_pv: Vec<String>,
    is_tinue: bool,
}

#[derive(Clone, Copy)]
pub enum TinueResponse {
    ExactResponse(Option<GameMove>),
    ValidResponse(Option<GameMove>),
    UnclearResponse(Option<GameMove>),
    PoorResponse(Option<GameMove>),
    Road,
    NoThreats(Option<GameMove>),
}

impl TinueResponse {
    pub fn inner(&self) -> Option<GameMove> {
        match self {
            TinueResponse::ExactResponse(mv)
            | TinueResponse::ValidResponse(mv)
            | TinueResponse::UnclearResponse(mv)
            | TinueResponse::PoorResponse(mv)
            | TinueResponse::NoThreats(mv) => *mv,
            TinueResponse::Road => None,
        }
    }
    pub fn is_terminal(&self) -> bool {
        match self {
            TinueResponse::Road | TinueResponse::NoThreats(_) => true,
            _ => false,
        }
    }
}

impl PuzzleState {
    fn new(puzzle_num: usize, data: &PuzzleData) -> Self {
        Self {
            puzzle_num,
            original_game: data.game_id,
            active_moves: Vec::new(),
            active_pv: data.pv.clone(),
            is_tinue: true,
        }
    }
    pub fn initial_pv(&self) -> &Vec<String> {
        &PUZZLES[self.puzzle_num].pv
    }
    pub fn initial_tps(&self) -> String {
        PUZZLES[self.puzzle_num].tps.clone()
    }
    pub fn initial_side(&self) -> topaz_tak::Color {
        let board = Board6::try_from_tps(&self.initial_tps()).unwrap();
        board.side_to_move()
    }
    pub fn build_board(&self) -> Board6 {
        // Todo eventually make this work with other sizes
        let mut board = Board6::try_from_tps(&PUZZLES[self.puzzle_num].tps).unwrap();
        for mv in self.active_moves.iter() {
            let mv = GameMove::try_from_ptn(mv, &board).unwrap();
            board.do_move(mv);
        }
        board
    }
    pub fn apply_move(&mut self, game_move: &str) {
        self.active_moves.push(game_move.to_string());
    }
    pub fn undo_player_move(&mut self) {
        self.active_moves.pop();
        self.active_moves.pop();
        if self.active_moves.len() == 0 {
            self.active_pv = self.initial_pv().clone();
        } else {
            self.active_pv = Vec::new();
        }
    }
    pub fn legal_moves(&self) -> Vec<String> {
        let mut board = self.build_board();
        let mut storage = Vec::new();
        if let Some(mv) = board.can_make_road(&mut storage, None) {
            return vec![mv.to_ptn::<Board6>()];
        }
        storage.clear();
        generate_all_moves(&board, &mut storage);
        board
            .get_tak_threats(&storage, None)
            .into_iter()
            .map(|x| x.to_ptn::<Board6>())
            .collect()
    }
    pub fn user_play_move(&mut self, ptn_move: &str) -> Option<TinueResponse> {
        let mut board = self.build_board();
        let mv = GameMove::try_from_ptn(ptn_move, &board)?;
        if !board.legal_move(mv) {
            return None;
        }
        board.do_move(mv);
        if let Some(_end) = board.game_result() {
            if board.road(self.initial_side()) {
                return Some(TinueResponse::Road);
            } else {
                return Some(TinueResponse::NoThreats(None));
            }
        }
        // Check if a road is threatened
        let mut storage = Vec::new();
        board.null_move();
        if board.can_make_road(&mut storage, None).is_none() {
            return Some(TinueResponse::NoThreats(None));
        }
        board.rev_null_move();
        let pv_move = self.active_pv.get(0).map(|x| x.as_str()).unwrap_or("");
        if self.is_tinue && pv_move == mv.to_ptn::<Board6>() {
            let reply = self
                .active_pv
                .get(1)
                .and_then(|x| GameMove::try_from_ptn(x, &board));
            self.active_pv.remove(0);
            if reply.is_some() {
                self.active_pv.remove(0);
            }
            return Some(TinueResponse::ExactResponse(reply));
        }
        // Fallback to search
        let mut search = TinueSearch::new(board)
            .quiet()
            .limit(250_000)
            .attacker(false);
        let tinue_res = search.is_tinue();
        self.is_tinue = tinue_res.unwrap_or(false);
        let pv = search.principal_variation();
        self.active_pv = pv.iter().skip(1).map(|x| x.to_ptn::<Board6>()).collect();
        let reply = pv.into_iter().next();
        match tinue_res {
            Some(true) => Some(TinueResponse::ValidResponse(reply)),
            Some(false) => Some(TinueResponse::PoorResponse(reply)),
            None => Some(TinueResponse::UnclearResponse(reply)),
        }
    }
    pub fn human_difficulty(&self) -> &'static str {
        PUZZLES[self.puzzle_num].human_difficulty()
    }
}

struct PuzzleData {
    game_id: usize,
    tps: String,
    pv: Vec<String>,
    difficulty: f32,
}

impl PuzzleData {
    fn human_difficulty(&self) -> &'static str {
        if self.difficulty < 1.0 {
            "Easy"
        } else if self.difficulty < 2.20 {
            "Medium"
        } else if self.difficulty < 3.5 {
            "Hard"
        } else {
            "Insane"
        }
    }
}
// Guess cutoffs: <1, 2.20, 3.5, 4.0

pub fn list_difficulties() {
    for puzzle in PUZZLES.iter() {
        if puzzle.difficulty >= 3.0 {
            println!("{}: {}, {:?}", puzzle.difficulty, puzzle.tps, puzzle.pv);
        }
        // println!("{}", puzzle.difficulty);
    }
}
