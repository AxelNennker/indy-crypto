use bn::BigNumber;
use errors::IndyCryptoError;

use pair::{
    GroupOrderElement,
    Pair,
    PointG1,
    PointG2,
};

use super::constants::*;
use cl::*;
use super::helpers::*;

use std::collections::{HashMap, HashSet};

pub struct Prover {}

impl Prover {
    pub fn new_master_secret() -> Result<MasterSecret, IndyCryptoError> {
        Ok(MasterSecret {
            ms: bn_rand(LARGE_MASTER_SECRET)?
        })
    }

    pub fn blinded_master_secret(pub_key: &IssuerPublicKey,
                                 ms: &MasterSecret) -> Result<(BlindedMasterSecret,
                                                               BlindedMasterSecretData), IndyCryptoError> {
        let blinded_primary_master_secret = Prover::_generate_blinded_primary_master_secret(&pub_key.p_key, &ms)?;

        let blinded_revocation_master_secret = match pub_key.r_key {
            Some(ref r_pk) => Some(Prover::_generate_blinded_revocation_master_secret(r_pk)?),
            _ => None
        };

        Ok((
            BlindedMasterSecret {
                u: blinded_primary_master_secret.u,
                ur: blinded_revocation_master_secret.as_ref().map(|d| d.ur)
            },
            BlindedMasterSecretData {
                v_prime: blinded_primary_master_secret.v_prime,
                vr_prime: blinded_revocation_master_secret.map(|d| d.vr_prime)
            }
        ))
    }

    fn _generate_blinded_primary_master_secret(p_pub_key: &IssuerPrimaryPublicKey,
                                               ms: &MasterSecret) -> Result<PrimaryBlindedMasterSecretData, IndyCryptoError> {
        let mut ctx = BigNumber::new_context()?;
        let v_prime = bn_rand(LARGE_VPRIME)?;

        let u = p_pub_key.s
            .mod_exp(&v_prime, &p_pub_key.n, Some(&mut ctx))?
            .mul(
                &p_pub_key.rms.mod_exp(&ms.ms, &p_pub_key.n, Some(&mut ctx))?,
                None
            )?
            .modulus(&p_pub_key.n, Some(&mut ctx))?;

        Ok(PrimaryBlindedMasterSecretData { u, v_prime })
    }

    fn _generate_blinded_revocation_master_secret(r_pub_key: &IssuerRevocationPublicKey) -> Result<RevocationBlindedMasterSecretData, IndyCryptoError> {
        let vr_prime = GroupOrderElement::new()?;
        let ur = r_pub_key.h2.mul(&vr_prime)?;

        Ok(RevocationBlindedMasterSecretData { ur, vr_prime })
    }

    pub fn process_claim_signature(claim: &mut ClaimSignature,
                                   blinded_master_secret_data: &BlindedMasterSecretData,
                                   pub_key: &IssuerPublicKey,
                                   r_reg: Option<&RevocationRegistryPublic>) -> Result<(), IndyCryptoError> {
        Prover::_process_primary_claim(&mut claim.p_claim, &blinded_master_secret_data.v_prime)?;

        if let (&mut Some(ref mut non_revocation_claim), Some(ref vr_prime), &Some(ref r_key), Some(ref r_reg)) = (&mut claim.r_claim,
                                                                                                                   blinded_master_secret_data.vr_prime,
                                                                                                                   &pub_key.r_key,
                                                                                                                   r_reg) {
            Prover::_process_non_revocation_claim(non_revocation_claim,
                                                  vr_prime,
                                                  &r_key,
                                                  r_reg)?;
        }
        Ok(())
    }

    fn _process_primary_claim(p_claim: &mut PrimaryClaimSignature,
                              v_prime: &BigNumber) -> Result<(), IndyCryptoError> {
        p_claim.v = v_prime.add(&p_claim.v)?;
        Ok(())
    }

    fn _process_non_revocation_claim(r_claim: &mut NonRevocationClaimSignature,
                                     vr_prime: &GroupOrderElement,
                                     r_pub_key: &IssuerRevocationPublicKey,
                                     r_reg: &RevocationRegistryPublic) -> Result<(), IndyCryptoError> {
        let r_cnxt_m2 = BigNumber::from_bytes(&r_claim.m2.to_bytes()?)?;
        r_claim.vr_prime_prime = vr_prime.add_mod(&r_claim.vr_prime_prime)?;
        Prover::_test_witness_credential(&r_claim, r_pub_key, r_reg, &r_cnxt_m2)?;
        Ok(())
    }

    fn _test_witness_credential(r_claim: &NonRevocationClaimSignature,
                                r_pub_key: &IssuerRevocationPublicKey,
                                r_reg: &RevocationRegistryPublic,
                                r_cnxt_m2: &BigNumber) -> Result<(), IndyCryptoError> {
        let z_calc = Pair::pair(&r_claim.witness.g_i, &r_reg.acc.acc)?
            .mul(&Pair::pair(&r_pub_key.g, &r_claim.witness.omega)?.inverse()?)?;
        if z_calc != r_reg.key.z {
            return Err(IndyCryptoError::InvalidStructure("Issuer is sending incorrect data".to_string()));
        }
        let pair_gg_calc = Pair::pair(&r_pub_key.pk.add(&r_claim.g_i)?, &r_claim.witness.sigma_i)?;
        let pair_gg = Pair::pair(&r_pub_key.g, &r_pub_key.g_dash)?;
        if pair_gg_calc != pair_gg {
            return Err(IndyCryptoError::InvalidStructure("Issuer is sending incorrect data".to_string()));
        }

        let m2 = GroupOrderElement::from_bytes(&r_cnxt_m2.to_bytes()?)?;

        let pair_h1 = Pair::pair(&r_claim.sigma, &r_pub_key.y.add(&r_pub_key.h_cap.mul(&r_claim.c)?)?)?;
        let pair_h2 = Pair::pair(
            &r_pub_key.h0
                .add(&r_pub_key.h1.mul(&m2)?)?
                .add(&r_pub_key.h2.mul(&r_claim.vr_prime_prime)?)?
                .add(&r_claim.g_i)?,
            &r_pub_key.h_cap
        )?;
        if pair_h1 != pair_h2 {
            return Err(IndyCryptoError::InvalidStructure("Issuer is sending incorrect data".to_string()));
        }

        Ok(())
    }

    pub fn new_proof_builder() -> Result<ProofBuilder, IndyCryptoError> {
        Ok(ProofBuilder {
            m1_tilde: bn_rand(LARGE_M2_TILDE)?,
            init_proofs: HashMap::new(),
            c_list: Vec::new(),
            tau_list: Vec::new()
        })
    }
}

#[derive(Debug)]
pub struct ProofBuilder {
    pub m1_tilde: BigNumber,
    pub init_proofs: HashMap<String, InitProof>,
    pub c_list: Vec<Vec<u8>>,
    pub tau_list: Vec<Vec<u8>>,
}

impl ProofBuilder {
    pub fn add_sub_proof_request(&mut self, key_id: &str, claim: &ClaimSignature, claim_values: ClaimValues, pub_key: &IssuerPublicKey,
                                 r_reg: Option<&RevocationRegistryPublic>, sub_proof_request: SubProofRequest, claim_schema: ClaimSchema) -> Result<(), IndyCryptoError> {
        let mut non_revoc_init_proof = None;
        let mut m2_tilde: Option<BigNumber> = None;

        if let (&Some(ref r_claim), &Some(ref r_reg), &Some(ref r_pub_key)) = (&claim.r_claim,
                                                                               &r_reg,
                                                                               &pub_key.r_key) {
            let proof = ProofBuilder::_init_non_revocation_proof(&mut r_claim.clone(), &r_reg, &r_pub_key)?;//TODO:FIXME

            self.c_list.extend_from_slice(&proof.as_c_list()?);
            self.tau_list.extend_from_slice(&proof.as_tau_list()?);
            m2_tilde = Some(group_element_to_bignum(&proof.tau_list_params.m2)?);
            non_revoc_init_proof = Some(proof);
        }

        let primary_init_proof = ProofBuilder::_init_primary_proof(&pub_key.p_key,
                                                                   &claim.p_claim,
                                                                   &claim_values,
                                                                   &claim_schema,
                                                                   &sub_proof_request,
                                                                   &self.m1_tilde,
                                                                   m2_tilde)?;

        self.c_list.extend_from_slice(&primary_init_proof.as_c_list()?);
        self.tau_list.extend_from_slice(&primary_init_proof.as_tau_list()?);

        let init_proof = InitProof {
            primary_init_proof,
            non_revoc_init_proof,
            claim_values,
            sub_proof_request,
            claim_schema
        };
        self.init_proofs.insert(key_id.to_owned(), init_proof);

        Ok(())
    }

    pub fn finalize(&mut self, nonce: &Nonce, ms: &MasterSecret) -> Result<Proof, IndyCryptoError> {
        let mut values: Vec<Vec<u8>> = Vec::new();
        values.extend_from_slice(&self.tau_list);
        values.extend_from_slice(&self.c_list);
        values.push(nonce.value.to_bytes()?);

        let c_h = get_hash_as_int(&mut values)?;

        let mut proofs: HashMap<String, SubProof> = HashMap::new();

        for (proof_claim_uuid, init_proof) in self.init_proofs.iter() {
            let mut non_revoc_proof: Option<NonRevocProof> = None;
            if let Some(ref non_revoc_init_proof) = init_proof.non_revoc_init_proof {
                non_revoc_proof = Some(ProofBuilder::_finalize_non_revocation_proof(&non_revoc_init_proof, &c_h)?);
            }

            let primary_proof = ProofBuilder::_finalize_primary_proof(&ms.ms,
                                                                      &init_proof.primary_init_proof,
                                                                      &c_h,
                                                                      &init_proof.claim_schema,
                                                                      &init_proof.claim_values,
                                                                      &init_proof.sub_proof_request)?;

            let proof = SubProof { primary_proof, non_revoc_proof };
            proofs.insert(proof_claim_uuid.to_owned(), proof);
        }

        let aggregated_proof = AggregatedProof { c_hash: c_h, c_list: self.c_list.clone() };

        Ok(Proof { proofs, aggregated_proof })
    }

    fn _init_primary_proof(pk: &IssuerPrimaryPublicKey, c1: &PrimaryClaimSignature, claim_values: &ClaimValues, claim_schema: &ClaimSchema,
                           sub_proof_request: &SubProofRequest, m1_t: &BigNumber,
                           m2_t: Option<BigNumber>) -> Result<PrimaryInitProof, IndyCryptoError> {
        let eq_proof = ProofBuilder::_init_eq_proof(&pk, c1, claim_schema, sub_proof_request, m1_t, m2_t)?;

        let mut ge_proofs: Vec<PrimaryPredicateGEInitProof> = Vec::new();
        for predicate in sub_proof_request.predicates.iter() {
            let ge_proof = ProofBuilder::_init_ge_proof(&pk, &eq_proof.m_tilde, claim_values, predicate)?;
            ge_proofs.push(ge_proof);
        }

        Ok(PrimaryInitProof { eq_proof, ge_proofs })
    }

    fn _init_non_revocation_proof(claim: &mut NonRevocationClaimSignature, rev_reg: &RevocationRegistryPublic, pkr: &IssuerRevocationPublicKey)
                                  -> Result<NonRevocInitProof, IndyCryptoError> {
        ProofBuilder::_update_non_revocation_claim(claim, &rev_reg.acc, &rev_reg.tails.tails_dash)?;

        let c_list_params = ProofBuilder::_gen_c_list_params(&claim)?;
        let proof_c_list = ProofBuilder::_create_c_list_values(&claim, &c_list_params, &pkr)?;

        let tau_list_params = ProofBuilder::_gen_tau_list_params()?;
        let proof_tau_list = ProofBuilder::create_tau_list_values(&pkr, &rev_reg.acc, &tau_list_params, &proof_c_list)?;

        Ok(NonRevocInitProof {
            c_list_params,
            tau_list_params,
            c_list: proof_c_list,
            tau_list: proof_tau_list
        })
    }

    fn _update_non_revocation_claim(claim: &mut NonRevocationClaimSignature,
                                    accum: &RevocationAccumulator, tails: &HashMap<u32, PointG2>)
                                    -> Result<(), IndyCryptoError> {
        if !accum.v.contains(&claim.i) {
            return Err(IndyCryptoError::InvalidState("Can not update Witness. Claim revoked.".to_string()));
        }

        if claim.witness.v != accum.v {
            let v_old_minus_new: HashSet<u32> =
                claim.witness.v.difference(&accum.v).cloned().collect();
            let mut omega_denom = PointG2::new_inf()?;
            for j in v_old_minus_new.iter() {
                omega_denom = omega_denom.add(
                    tails.get(&(accum.max_claim_num + 1 - j + claim.i))
                        .ok_or(IndyCryptoError::InvalidStructure(format!("Key not found {} in tails", accum.max_claim_num + 1 - j + claim.i)))?)?;
            }
            let mut omega_num = PointG2::new_inf()?;
            let mut new_omega: PointG2 = claim.witness.omega.clone();
            for j in v_old_minus_new.iter() {
                omega_num = omega_num.add(
                    tails.get(&(accum.max_claim_num + 1 - j + claim.i))
                        .ok_or(IndyCryptoError::InvalidStructure(format!("Key not found {} in tails", accum.max_claim_num + 1 - j + claim.i)))?)?;
                new_omega = new_omega.add(
                    &omega_num.sub(&omega_denom)?
                )?;
            }

            claim.witness.v = accum.v.clone();
            claim.witness.omega = new_omega;
        }

        Ok(())
    }

    fn _init_eq_proof(pk: &IssuerPrimaryPublicKey, c1: &PrimaryClaimSignature, claim_schema: &ClaimSchema, sub_proof_request: &SubProofRequest,
                      m1_tilde: &BigNumber, m2_t: Option<BigNumber>) -> Result<PrimaryEqualInitProof, IndyCryptoError> {
        let mut ctx = BigNumber::new_context()?;

        let m2_tilde = m2_t.unwrap_or(bn_rand(LARGE_MVECT)?);

        let r = bn_rand(LARGE_VPRIME)?;
        let e_tilde = bn_rand(LARGE_ETILDE)?;
        let v_tilde = bn_rand(LARGE_VTILDE)?;

        let unrevealed_attrs: HashSet<String> =
            claim_schema.attrs
                .difference(&sub_proof_request.revealed_attrs)
                .map(|attr| attr.clone())
                .collect::<HashSet<String>>();

        let m_tilde = get_mtilde(&unrevealed_attrs)?;

        let a_prime = pk.s
            .mod_exp(&r, &pk.n, Some(&mut ctx))?
            .mul(&c1.a, Some(&mut ctx))?
            .modulus(&pk.n, Some(&mut ctx))?;

        let large_e_start = BigNumber::from_dec(&LARGE_E_START.to_string())?;

        let v_prime = c1.v.sub(
            &c1.e.mul(&r, Some(&mut ctx))?
        )?;

        let e_prime = c1.e.sub(
            &BigNumber::from_dec("2")?.exp(&large_e_start, Some(&mut ctx))?
        )?;

        let t = calc_teq(&pk, &a_prime, &e_tilde, &v_tilde, &m_tilde, &m1_tilde,
                         &m2_tilde, &unrevealed_attrs)?;

        Ok(PrimaryEqualInitProof {
            a_prime,
            t,
            e_tilde,
            e_prime,
            v_tilde,
            v_prime,
            m_tilde,
            m1_tilde: m1_tilde.clone()?,
            m2_tilde: m2_tilde.clone()?,
            m2: c1.m_2.clone()?
        })
    }

    fn _init_ge_proof(pk: &IssuerPrimaryPublicKey, mtilde: &HashMap<String, BigNumber>,
                      claim_values: &ClaimValues, predicate: &Predicate)
                      -> Result<PrimaryPredicateGEInitProof, IndyCryptoError> {
        let mut ctx = BigNumber::new_context()?;
        let (k, value) = (&predicate.attr_name, predicate.value);

        let attr_value = claim_values.attrs_values.get(&k[..])
            .ok_or(IndyCryptoError::InvalidStructure(format!("Value by key '{}' not found in claim_values", k)))?
            .to_dec()?
            .parse::<i32>()
            .map_err(|_| IndyCryptoError::InvalidStructure(format!("Value by key '{}' has invalid format", k)))?;

        let delta: i32 = attr_value - value;

        if delta < 0 {
            return Err(IndyCryptoError::InvalidStructure("Predicate is not satisfied".to_string()));
        }

        let u = four_squares(delta)?;

        let mut r: HashMap<String, BigNumber> = HashMap::new();
        let mut t: HashMap<String, BigNumber> = HashMap::new();
        let mut c_list: Vec<BigNumber> = Vec::new();

        for i in 0..ITERATION {
            let cur_u = u.get(&i.to_string())
                .ok_or(IndyCryptoError::InvalidStructure(format!("Value by key '{}' not found in u1", i)))?;

            let cur_r = bn_rand(LARGE_VPRIME)?;

            let cut_t = pk.z
                .mod_exp(&cur_u, &pk.n, Some(&mut ctx))?
                .mul(
                    &pk.s.mod_exp(&cur_r, &pk.n, Some(&mut ctx))?,
                    Some(&mut ctx)
                )?
                .modulus(&pk.n, Some(&mut ctx))?;

            r.insert(i.to_string(), cur_r);
            t.insert(i.to_string(), cut_t.clone()?);
            c_list.push(cut_t)
        }

        let r_delta = bn_rand(LARGE_VPRIME)?;

        let t_delta = pk.z
            .mod_exp(&BigNumber::from_dec(&delta.to_string())?, &pk.n, Some(&mut ctx))?
            .mul(
                &pk.s.mod_exp(&r_delta, &pk.n, Some(&mut ctx))?,
                Some(&mut ctx)
            )?
            .modulus(&pk.n, Some(&mut ctx))?;

        r.insert("DELTA".to_string(), r_delta);
        t.insert("DELTA".to_string(), t_delta.clone()?);
        c_list.push(t_delta);

        let mut u_tilde: HashMap<String, BigNumber> = HashMap::new();
        let mut r_tilde: HashMap<String, BigNumber> = HashMap::new();

        for i in 0..ITERATION {
            u_tilde.insert(i.to_string(), bn_rand(LARGE_UTILDE)?);
            r_tilde.insert(i.to_string(), bn_rand(LARGE_RTILDE)?);
        }

        r_tilde.insert("DELTA".to_string(), bn_rand(LARGE_RTILDE)?);
        let alpha_tilde = bn_rand(LARGE_ALPHATILDE)?;

        let mj = mtilde.get(&k[..])
            .ok_or(IndyCryptoError::InvalidStructure(format!("Value by key '{}' not found in eq_proof.mtilde", k)))?;

        let tau_list = calc_tge(&pk, &u_tilde, &r_tilde, &mj, &alpha_tilde, &t)?;

        Ok(PrimaryPredicateGEInitProof {
            c_list,
            tau_list,
            u,
            u_tilde,
            r,
            r_tilde,
            alpha_tilde,
            predicate: predicate.clone(),
            t
        })
    }

    fn _finalize_eq_proof(ms: &BigNumber, init_proof: &PrimaryEqualInitProof, c_h: &BigNumber,
                          claim_schema: &ClaimSchema, claim_values: &ClaimValues, sub_proof_request: &SubProofRequest)
                          -> Result<PrimaryEqualProof, IndyCryptoError> {
        let mut ctx = BigNumber::new_context()?;

        let e = c_h
            .mul(&init_proof.e_prime, Some(&mut ctx))?
            .add(&init_proof.e_tilde)?;

        let v = c_h
            .mul(&init_proof.v_prime, Some(&mut ctx))?
            .add(&init_proof.v_tilde)?;

        let mut m: HashMap<String, BigNumber> = HashMap::new();

        let unrevealed_attrs: HashSet<String> =
            claim_schema.attrs
                .difference(&sub_proof_request.revealed_attrs)
                .map(|attr| attr.clone())
                .collect::<HashSet<String>>();

        for k in unrevealed_attrs.iter() {
            let cur_mtilde = init_proof.m_tilde.get(k)
                .ok_or(IndyCryptoError::InvalidStructure(format!("Value by key '{}' not found in init_proof.mtilde", k)))?;

            let cur_val = claim_values.attrs_values.get(k)
                .ok_or(IndyCryptoError::InvalidStructure(format!("Value by key '{}' not found in attributes_values", k)))?;

            let val = c_h
                .mul(&cur_val, Some(&mut ctx))?
                .add(&cur_mtilde)?;

            m.insert(k.clone(), val);
        }

        let m1 = c_h
            .mul(&ms, Some(&mut ctx))?
            .add(&init_proof.m1_tilde)?;

        let m2 = c_h
            .mul(&init_proof.m2, Some(&mut ctx))?
            .add(&init_proof.m2_tilde)?;


        let mut revealed_attrs_with_values: HashMap<String, BigNumber> = HashMap::new();

        for attr in sub_proof_request.revealed_attrs.iter() {
            revealed_attrs_with_values.insert(
                attr.clone(),
                claim_values.attrs_values
                    .get(attr)
                    .ok_or(IndyCryptoError::InvalidStructure(format!("Encoded value not found")))?
                    .clone()?,
            );
        }

        Ok(PrimaryEqualProof {
            revealed_attrs: revealed_attrs_with_values,
            a_prime: init_proof.a_prime.clone()?,
            e,
            v,
            m,
            m1,
            m2
        })
    }

    fn _finalize_ge_proof(c_h: &BigNumber, init_proof: &PrimaryPredicateGEInitProof,
                          eq_proof: &PrimaryEqualProof) -> Result<PrimaryPredicateGEProof, IndyCryptoError> {
        let mut ctx = BigNumber::new_context()?;
        let mut u: HashMap<String, BigNumber> = HashMap::new();
        let mut r: HashMap<String, BigNumber> = HashMap::new();
        let mut urproduct = BigNumber::new()?;

        for i in 0..ITERATION {
            let cur_utilde = init_proof.u_tilde.get(&i.to_string())
                .ok_or(IndyCryptoError::InvalidStructure(format!("Value by key '{}' not found in init_proof.u_tilde", i)))?;
            let cur_u = init_proof.u.get(&i.to_string())
                .ok_or(IndyCryptoError::InvalidStructure(format!("Value by key '{}' not found in init_proof.u", i)))?;
            let cur_rtilde = init_proof.r_tilde.get(&i.to_string())
                .ok_or(IndyCryptoError::InvalidStructure(format!("Value by key '{}' not found in init_proof.r_tilde", i)))?;
            let cur_r = init_proof.r.get(&i.to_string())
                .ok_or(IndyCryptoError::InvalidStructure(format!("Value by key '{}' not found in init_proof.r", i)))?;

            let new_u: BigNumber = c_h
                .mul(&cur_u, Some(&mut ctx))?
                .add(&cur_utilde)?;
            let new_r: BigNumber = c_h
                .mul(&cur_r, Some(&mut ctx))?
                .add(&cur_rtilde)?;

            u.insert(i.to_string(), new_u);
            r.insert(i.to_string(), new_r);

            urproduct = cur_u
                .mul(&cur_r, Some(&mut ctx))?
                .add(&urproduct)?;

            let cur_rtilde_delta = init_proof.r_tilde.get("DELTA")
                .ok_or(IndyCryptoError::InvalidStructure(format!("Value by key '{}' not found in init_proof.r_tilde", "DELTA")))?;
            let cur_r_delta = init_proof.r.get("DELTA")
                .ok_or(IndyCryptoError::InvalidStructure(format!("Value by key '{}' not found in init_proof.r", "DELTA")))?;

            let new_delta = c_h
                .mul(&cur_r_delta, Some(&mut ctx))?
                .add(&cur_rtilde_delta)?;

            r.insert("DELTA".to_string(), new_delta);
        }

        let r_delta = init_proof.r.get("DELTA")
            .ok_or(IndyCryptoError::InvalidStructure(format!("Value by key '{}' not found in init_proof.r", "DELTA")))?;

        let alpha = r_delta
            .sub(&urproduct)?
            .mul(&c_h, Some(&mut ctx))?
            .add(&init_proof.alpha_tilde)?;

        let mj = eq_proof.m.get(&init_proof.predicate.attr_name)
            .ok_or(IndyCryptoError::InvalidStructure(format!("Value by key '{}' not found in eq_proof.m", init_proof.predicate.attr_name)))?;

        Ok(PrimaryPredicateGEProof {
            u,
            r,
            mj: mj.clone()?,
            alpha,
            t: clone_bignum_map(&init_proof.t)?,
            predicate: init_proof.predicate.clone()
        })
    }

    fn _finalize_primary_proof(ms: &BigNumber, init_proof: &PrimaryInitProof, c_h: &BigNumber,
                               claim_schema: &ClaimSchema, claim_values: &ClaimValues, sub_proof_request: &SubProofRequest)
                               -> Result<PrimaryProof, IndyCryptoError> {
        info!(target: "anoncreds_service", "Prover finalize proof -> start");

        let eq_proof = ProofBuilder::_finalize_eq_proof(ms, &init_proof.eq_proof, c_h, claim_schema, claim_values, sub_proof_request)?;
        let mut ge_proofs: Vec<PrimaryPredicateGEProof> = Vec::new();

        for init_ge_proof in init_proof.ge_proofs.iter() {
            let ge_proof = ProofBuilder::_finalize_ge_proof(c_h, init_ge_proof, &eq_proof)?;
            ge_proofs.push(ge_proof);
        }

        info!(target: "anoncreds_service", "Prover finalize proof -> done");

        Ok(PrimaryProof { eq_proof, ge_proofs })
    }

    fn _gen_c_list_params(claim: &NonRevocationClaimSignature) -> Result<NonRevocProofXList, IndyCryptoError> {
        let rho = GroupOrderElement::new()?;
        let r = GroupOrderElement::new()?;
        let r_prime = GroupOrderElement::new()?;
        let r_prime_prime = GroupOrderElement::new()?;
        let r_prime_prime_prime = GroupOrderElement::new()?;
        let o = GroupOrderElement::new()?;
        let o_prime = GroupOrderElement::new()?;
        let m = rho.mul_mod(&claim.c)?;
        let m_prime = r.mul_mod(&r_prime_prime)?;
        let t = o.mul_mod(&claim.c)?;
        let t_prime = o_prime.mul_mod(&r_prime_prime)?;
        let m2 = GroupOrderElement::from_bytes(&claim.m2.to_bytes()?)?;

        Ok(NonRevocProofXList {
            rho,
            r,
            r_prime,
            r_prime_prime,
            r_prime_prime_prime,
            o,
            o_prime,
            m,
            m_prime,
            t,
            t_prime,
            m2,
            s: claim.vr_prime_prime,
            c: claim.c
        })
    }

    fn _create_c_list_values(claim: &NonRevocationClaimSignature, params: &NonRevocProofXList,
                             pkr: &IssuerRevocationPublicKey) -> Result<NonRevocProofCList, IndyCryptoError> {
        let e = pkr.h
            .mul(&params.rho)?
            .add(
                &pkr.htilde.mul(&params.o)?
            )?;

        let d = pkr.g
            .mul(&params.r)?
            .add(
                &pkr.htilde.mul(&params.o_prime)?
            )?;

        let a = claim.sigma
            .add(
                &pkr.htilde.mul(&params.rho)?
            )?;

        let g = claim.g_i
            .add(
                &pkr.htilde.mul(&params.r)?
            )?;

        let w = claim.witness.omega
            .add(
                &pkr.h_cap.mul(&params.r_prime)?
            )?;

        let s = claim.witness.sigma_i
            .add(
                &pkr.h_cap.mul(&params.r_prime_prime)?
            )?;

        let u = claim.witness.u_i
            .add(
                &pkr.h_cap.mul(&params.r_prime_prime_prime)?
            )?;

        Ok(NonRevocProofCList {
            e,
            d,
            a,
            g,
            w,
            s,
            u
        })
    }

    fn _gen_tau_list_params() -> Result<NonRevocProofXList, IndyCryptoError> {
        Ok(NonRevocProofXList {
            rho: GroupOrderElement::new()?,
            r: GroupOrderElement::new()?,
            r_prime: GroupOrderElement::new()?,
            r_prime_prime: GroupOrderElement::new()?,
            r_prime_prime_prime: GroupOrderElement::new()?,
            o: GroupOrderElement::new()?,
            o_prime: GroupOrderElement::new()?,
            m: GroupOrderElement::new()?,
            m_prime: GroupOrderElement::new()?,
            t: GroupOrderElement::new()?,
            t_prime: GroupOrderElement::new()?,
            m2: GroupOrderElement::new()?,
            s: GroupOrderElement::new()?,
            c: GroupOrderElement::new()?
        })
    }

    fn _finalize_non_revocation_proof(init_proof: &NonRevocInitProof, c_h: &BigNumber) -> Result<NonRevocProof, IndyCryptoError> {
        info!(target: "anoncreds_service", "Prover finalize non-revocation proof -> start");

        let ch_num_z = bignum_to_group_element(&c_h)?;
        let mut x_list: Vec<GroupOrderElement> = Vec::new();

        for (x, y) in init_proof.tau_list_params.as_list()?.iter().zip(init_proof.c_list_params.as_list()?.iter()) {
            x_list.push(x.add_mod(
                &ch_num_z.mul_mod(&y)?.mod_neg()?
            )?);
        }

        info!(target: "anoncreds_service", "Prover finalize non-revocation proof -> done");

        Ok(NonRevocProof {
            x_list: NonRevocProofXList::from_list(x_list),
            c_list: init_proof.c_list.clone()
        })
    }

    pub fn create_tau_list_values(pk_r: &IssuerRevocationPublicKey, accumulator: &RevocationAccumulator,
                                  params: &NonRevocProofXList, proof_c: &NonRevocProofCList) -> Result<NonRevocProofTauList, IndyCryptoError> {
        let t1 = pk_r.h.mul(&params.rho)?.add(&pk_r.htilde.mul(&params.o)?)?;
        let mut t2 = proof_c.e.mul(&params.c)?
            .add(&pk_r.h.mul(&params.m.mod_neg()?)?)?
            .add(&pk_r.htilde.mul(&params.t.mod_neg()?)?)?;
        if t2.is_inf()? {
            t2 = PointG1::new_inf()?;
        }
        let t3 = Pair::pair(&proof_c.a, &pk_r.h_cap)?.pow(&params.c)?
            .mul(&Pair::pair(&pk_r.htilde, &pk_r.h_cap)?.pow(&params.r)?)?
            .mul(&Pair::pair(&pk_r.htilde, &pk_r.y)?.pow(&params.rho)?
                .mul(&Pair::pair(&pk_r.htilde, &pk_r.h_cap)?.pow(&params.m)?)?
                .mul(&Pair::pair(&pk_r.h1, &pk_r.h_cap)?.pow(&params.m2)?)?
                .mul(&Pair::pair(&pk_r.h2, &pk_r.h_cap)?.pow(&params.s)?)?.inverse()?)?;
        let t4 = Pair::pair(&pk_r.htilde, &accumulator.acc)?
            .pow(&params.r)?
            .mul(&Pair::pair(&pk_r.g.neg()?, &pk_r.h_cap)?.pow(&params.r_prime)?)?;
        let t5 = pk_r.g.mul(&params.r)?.add(&pk_r.htilde.mul(&params.o_prime)?)?;
        let mut t6 = proof_c.d.mul(&params.r_prime_prime)?
            .add(&pk_r.g.mul(&params.m_prime.mod_neg()?)?)?
            .add(&pk_r.htilde.mul(&params.t_prime.mod_neg()?)?)?;
        if t6.is_inf()? {
            t6 = PointG1::new_inf()?;
        }
        let t7 = Pair::pair(&pk_r.pk.add(&proof_c.g)?, &pk_r.h_cap)?.pow(&params.r_prime_prime)?
            .mul(&Pair::pair(&pk_r.htilde, &pk_r.h_cap)?.pow(&params.m_prime.mod_neg()?)?)?
            .mul(&Pair::pair(&pk_r.htilde, &proof_c.s)?.pow(&params.r)?)?;
        let t8 = Pair::pair(&pk_r.htilde, &pk_r.u)?.pow(&params.r)?
            .mul(&Pair::pair(&pk_r.g.neg()?, &pk_r.h_cap)?.pow(&params.r_prime_prime_prime)?)?;

        Ok(NonRevocProofTauList {
            t1,
            t2,
            t3,
            t4,
            t5,
            t6,
            t7,
            t8
        })
    }

    pub fn create_tau_list_expected_values(pk_r: &IssuerRevocationPublicKey, accumulator: &RevocationAccumulator,
                                           accum_pk: &RevocationAccumulatorPublicKey, proof_c: &NonRevocProofCList) -> Result<NonRevocProofTauList, IndyCryptoError> {
        let t1 = proof_c.e;
        let t2 = PointG1::new_inf()?;
        let t3 = Pair::pair(&pk_r.h0.add(&proof_c.g)?, &pk_r.h_cap)?
            .mul(&Pair::pair(&proof_c.a, &pk_r.y)?.inverse()?)?;
        let t4 = Pair::pair(&proof_c.g, &accumulator.acc)?
            .mul(&Pair::pair(&pk_r.g, &proof_c.w)?.mul(&accum_pk.z)?.inverse()?)?;
        let t5 = proof_c.d;
        let t6 = PointG1::new_inf()?;
        let t7 = Pair::pair(&pk_r.pk.add(&proof_c.g)?, &proof_c.s)?
            .mul(&Pair::pair(&pk_r.g, &pk_r.g_dash)?.inverse()?)?;
        let t8 = Pair::pair(&proof_c.g, &pk_r.u)?
            .mul(&Pair::pair(&pk_r.g, &proof_c.u)?.inverse()?)?;

        Ok(NonRevocProofTauList {
            t1,
            t2,
            t3,
            t4,
            t5,
            t6,
            t7,
            t8
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::issuer;

    #[test]
    fn generate_master_secret_works() {
        let ms = Prover::new_master_secret().unwrap();
        assert_eq!(ms.ms.to_dec().unwrap(), mocks::master_secret().ms.to_dec().unwrap());
    }

    #[test]
    fn generate_blinded_primary_master_secret_works() {
        let pk = issuer::mocks::issuer_primary_public_key();
        let ms = mocks::master_secret();

        let blinded_primary_master_secret = Prover::_generate_blinded_primary_master_secret(&pk, &ms).unwrap();
        assert_eq!(blinded_primary_master_secret, mocks::primary_blinded_master_secret_data());
    }

    #[test]
    fn generate_blinded_revocation_master_secret_works() {
        let r_pk = issuer::mocks::revocation_pub_key();
        Prover::_generate_blinded_revocation_master_secret(&r_pk).unwrap();
    }

    #[test]
    fn generate_blinded_master_secret_works() {
        let pk = issuer::mocks::issuer_public_key();
        let ms = super::mocks::master_secret();

        let (blinded_master_secret, blinded_master_secret_data) = Prover::blinded_master_secret(&pk, &ms).unwrap();

        assert_eq!(blinded_master_secret.u, mocks::primary_blinded_master_secret_data().u);
        assert_eq!(blinded_master_secret_data.v_prime, mocks::primary_blinded_master_secret_data().v_prime);
        assert!(blinded_master_secret.ur.is_some());
        assert!(blinded_master_secret_data.vr_prime.is_some());
    }

    #[test]
    fn process_primary_claim_works() {
        let mut claim = issuer::mocks::primary_claim();
        let v_prime = mocks::primary_blinded_master_secret_data().v_prime;

        Prover::_process_primary_claim(&mut claim, &v_prime).unwrap();

        assert_eq!(mocks::primary_claim(), claim);
    }

    #[test]
    fn process_claim_works() {
        let mut claim = issuer::mocks::claim();
        let pk = issuer::mocks::issuer_public_key();
        let blinded_master_secret_data = mocks::blinded_master_secret_data();

        Prover::process_claim_signature(&mut claim, &blinded_master_secret_data, &pk, None).unwrap();

        assert_eq!(mocks::primary_claim(), claim.p_claim);
    }

    #[test]
    fn init_eq_proof_works() {
        let pk = issuer::mocks::issuer_primary_public_key();
        let claim_schema = issuer::mocks::claim_schema();
        let claim = mocks::primary_claim();
        let sub_proof_request = mocks::sub_proof_request();
        let m1_t = mocks::m1_t();

        let init_eq_proof = ProofBuilder::_init_eq_proof(&pk,
                                                         &claim,
                                                         &claim_schema,
                                                         &sub_proof_request,
                                                         &m1_t,
                                                         None).unwrap();

        assert_eq!(mocks::primary_equal_init_proof(), init_eq_proof);
    }

    #[test]
    fn init_ge_proof_works() {
        let pk = issuer::mocks::issuer_primary_public_key();
        let init_eq_proof = mocks::primary_equal_init_proof();
        let predicate = mocks::predicate();
        let claim_schema = issuer::mocks::claim_values();

        let init_ge_proof = ProofBuilder::_init_ge_proof(&pk,
                                                         &init_eq_proof.m_tilde,
                                                         &claim_schema,
                                                         &predicate).unwrap();

        assert_eq!(mocks::primary_ge_init_proof(), init_ge_proof);
    }

    #[test]
    fn init_primary_proof_works() {
        let pk = issuer::mocks::issuer_primary_public_key();
        let claim_schema = issuer::mocks::claim_schema();
        let claim = mocks::claim();
        let m1_t = mocks::m1_t();
        let claim_values = issuer::mocks::claim_values();
        let sub_proof_request = mocks::sub_proof_request();

        let init_proof = ProofBuilder::_init_primary_proof(&pk,
                                                           &claim.p_claim,
                                                           &claim_values,
                                                           &claim_schema,
                                                           &sub_proof_request,
                                                           &m1_t,
                                                           None).unwrap();
        assert_eq!(mocks::primary_init_proof(), init_proof);
    }

    #[test]
    fn finalize_eq_proof_works() {
        let ms = mocks::master_secret();
        let c_h = mocks::aggregated_proof().c_hash;
        let init_proof = mocks::primary_equal_init_proof();
        let claim_values = issuer::mocks::claim_values();
        let claim_schema = issuer::mocks::claim_schema();
        let sub_proof_request = mocks::sub_proof_request();

        let eq_proof = ProofBuilder::_finalize_eq_proof(&ms.ms,
                                                        &init_proof,
                                                        &c_h,
                                                        &claim_schema,
                                                        &claim_values,
                                                        &sub_proof_request).unwrap();

        assert_eq!(mocks::eq_proof(), eq_proof);
    }

    #[test]
    fn finalize_ge_proof_works() {
        let c_h = mocks::aggregated_proof().c_hash;
        let ge_proof = mocks::primary_ge_init_proof();
        let eq_proof = mocks::eq_proof();

        let ge_proof = ProofBuilder::_finalize_ge_proof(&c_h,
                                                        &ge_proof,
                                                        &eq_proof).unwrap();
        assert_eq!(mocks::ge_proof(), ge_proof);
    }

    #[test]
    fn finalize_primary_proof_works() {
        let proof = mocks::primary_init_proof();
        let ms = mocks::master_secret();
        let c_h = mocks::aggregated_proof().c_hash;
        let claim_schema = issuer::mocks::claim_schema();
        let claim_values = issuer::mocks::claim_values();
        let sub_proof_request = mocks::sub_proof_request();

        let proof = ProofBuilder::_finalize_primary_proof(&ms.ms,
                                                          &proof,
                                                          &c_h,
                                                          &claim_schema,
                                                          &claim_values,
                                                          &sub_proof_request).unwrap();

        assert_eq!(mocks::primary_proof(), proof);
    }

    #[test]
    fn test_witness_credential_works() {
        let mut r_claim = issuer::mocks::revocation_claim();
        let r_key = issuer::mocks::revocation_pub_key();
        let pub_rev_reg = issuer::mocks::revocation_reg_public();
        let r_cnxt_m2 = issuer::mocks::r_cnxt_m2();

        Prover::_test_witness_credential(&mut r_claim, &r_key, &pub_rev_reg, &r_cnxt_m2).unwrap();
    }

    #[test]
    fn test_c_and_tau_list() {
        let r_claim = issuer::mocks::revocation_claim();
        let r_key = issuer::mocks::revocation_pub_key();
        let pub_rev_reg = issuer::mocks::revocation_reg_public();

        let c_list_params = ProofBuilder::_gen_c_list_params(&r_claim).unwrap();

        let proof_c_list = ProofBuilder::_create_c_list_values(&r_claim, &c_list_params, &r_key).unwrap();

        let proof_tau_list = ProofBuilder::create_tau_list_values(&r_key, &pub_rev_reg.acc,
                                                                  &c_list_params, &proof_c_list).unwrap();

        let proof_tau_list_calc = ProofBuilder::create_tau_list_expected_values(&r_key,
                                                                                &pub_rev_reg.acc,
                                                                                &pub_rev_reg.key,
                                                                                &proof_c_list).unwrap();

        assert_eq!(proof_tau_list.as_slice().unwrap(), proof_tau_list_calc.as_slice().unwrap());
    }
}

pub mod mocks {
    use std::iter::FromIterator;
    use super::*;
    use super::super::issuer;

    pub const PROVER_DID: &'static str = "CnEDk9HrMnmiHXEV1WFgbVCRteYnPqsJwrTdcZaNhFVW";

    pub fn master_secret() -> MasterSecret {
        MasterSecret {
            ms: BigNumber::from_dec("21578029250517794450984707538122537192839006240802068037273983354680998203845").unwrap()
        }
    }

    pub fn blinded_master_secret_data() -> BlindedMasterSecretData {
        BlindedMasterSecretData {
            v_prime: primary_blinded_master_secret_data().v_prime,
            vr_prime: Some(GroupOrderElement::new().unwrap())
        }
    }

    pub fn primary_blinded_master_secret_data() -> PrimaryBlindedMasterSecretData {
        PrimaryBlindedMasterSecretData {
            u: BigNumber::from_dec("62131613458491212647450749026110557315107248063999634018939493990510661547774785043368606327349972438752553705268389551695956681591088513470965951022916188426635920785711858270846103151952143962999882605158874187727930917543065819603904033232476213318716946483165845049857055843524772096401162219219325766151823342237298870123405045483888204774734861333194064636771376483246576553005091050395021110616183024926509075608486405908792354917392247618138553245001668496721002592137124689913074323672408089937272809493673139956967625985778946668553397964410414804110497637727146455394436693696946473591314513302670305281967").unwrap(),
            v_prime: BigNumber::from_dec("1921424195886158938744777125021406748763985122590553448255822306242766229793715475428833504725487921105078008192433858897449555181018215580757557939320974389877538474522876366787859030586130885280724299566241892352485632499791646228580480458657305087762181033556428779333220803819945703716249441372790689501824842594015722727389764537806761583087605402039968357991056253519683582539703803574767702877615632257021995763302779502949501243649740921598491994352181379637769188829653918416991301420900374928589100515793950374255826572066003334385555085983157359122061582085202490537551988700484875690854200826784921400257387622318582276996322436").unwrap()
        }
    }

    pub fn claim() -> ClaimSignature {
        ClaimSignature {
            p_claim: primary_claim(),
            r_claim: Some(issuer::mocks::revocation_claim())
        }
    }

    pub fn m1_t() -> BigNumber {
        BigNumber::from_dec("67940925789970108743024738273926421512152745397724199848594503731042154269417576665420030681245389493783225644817826683796657351721363490290016166310023506339911751676800452438014771736117676826911321621579680668201191205819012441197794443970687648330757835198888257781967404396196813475280544039772512800509").unwrap()
    }

    pub fn primary_claim() -> PrimaryClaimSignature {
        PrimaryClaimSignature {
            m_2: BigNumber::from_dec("52860447312636183767369476481903349046618423276302392993759146262753859184069").unwrap(),
            a: BigNumber::from_dec("42346013283891624356845593609972612732277099460808703423355031452333398587460249347554583456785568352770358601256261145961117826420380272416387829159844410014785426604095869040832710421201960517786309019912783315267089765677766811003940302066320412180304382053168981384009266136660799893688028251946293637550958281867142669430870313657294704058632411088409642095191207839719892618500769362572243815788078229669302285658706213213138796427929865832385051039056434673653356180049577892616238217062562161769551903089986522565357147971417777037476294925507410600661934181377140384456587982053682992622578078710737752284145").unwrap(),
            e: BigNumber::from_dec("259344723055062059907025491480697571938277889515152306249728583105665800713306759149981690559193987143012367913206299323899696942213235956742930201588264091397308910346117473868881").unwrap(),
            v: BigNumber::from_dec("6620937836014079781509458870800001917950459774302786434315639456568768602266735503527631640833663968617512880802104566048179854406925811731340920442625764155409951969854303612644127544973467090784833169581477025096651956458587024481106269073426545688878633368395090950721246745797130514914475184220252785922714892764536041334549342283500382915967329086709002330282037812607548379718641877595592743676836398647524633348205332354808351273389207425490367080293557186321576642355686995967422099839906367044852871358174711678743078106239862383119503287568833606375474359241383490799700740580296717320354647238288294827855343155547056851646090370313395520915221874011198982966904484363631910557996205942678772502957389321620232931357572315089162587705606682143499451357592399858038685832965830759409094928957246320485487746463").unwrap()
        }
    }

    pub fn primary_init_proof() -> PrimaryInitProof {
        PrimaryInitProof {
            eq_proof: primary_equal_init_proof(),
            ge_proofs: vec![primary_ge_init_proof()]
        }
    }

    pub fn primary_equal_init_proof() -> PrimaryEqualInitProof {
        let a_prime = BigNumber::from_dec("55843178746788520435119921377390286268231906459093621159092786036045167470853237525750282569899268449257074522170415569390034716112368052014334531932423576902914762587842041027851281255714993168169397606683305211998762679155888831040910409096324726866578032630340975792746967149464040920396539558743125725962501042923116270042003617984521164987313533431008960902795014252204100220831622879715088891023369836472708006594040942575574164603258335948975612936260979474698634607068185834612498203540495530063764980572148330080073950973355083221837170619929706589683883912936747392721735783231955174948213742908010137510059").unwrap();
        let t = BigNumber::from_dec("48973018977144055944669519989481642869215488929062998734901463664459756294241601155321632240182242741351527618397570977014997144652907498803199568473576436659761897383165560744484277311963229373730027481518334564056910664855021973780670809391869976972256366927266299588055669244439406136391755306452039493468720569572147236503337456803407016560119357108968937752154999009135739288690305269248049643647114493574609913368697724563902578317988938688111455603289064969925062487403801982788918294180433456215967608275862853309081415780863179218684897030156771304698466844021124396633227553157619577905096144602889860383546").unwrap();
        let e_tilde = BigNumber::from_dec("162083298053730499878539835193560156486733663622707027216327685550780519347628838870322946818623352681120371349972731968874009673965057322").unwrap();
        let e_prime = BigNumber::from_dec("524456141360955985047633523128638545").unwrap();
        let v_tilde = BigNumber::from_dec("241132863422049783305938184561371219250127488499746090592218003869595412171810997360214885239402274273939963489505434726467041932541499422544431299362364797699330176612923593931231233163363211565697860685967381420219969754969010598350387336530924879073366177641099382257720898488467175132844984811431059686249020737675861448309521855120928434488546976081485578773933300425198911646071284164884533755653094354378714645351464093907890440922615599556866061098147921890790915215227463991346847803620736586839786386846961213073783437136210912924729098636427160258710930323242639624389905049896225019051952864864612421360643655700799102439682797806477476049234033513929028472955119936073490401848509891547105031112859155855833089675654686301183778056755431562224990888545742379494795601542482680006851305864539769704029428620446639445284011289708313620219638324467338840766574612783533920114892847440641473989502440960354573501").unwrap();
        let v_prime = BigNumber::from_dec("6122626610060688577826028713229499074477199356382901788064599481139201841946675307459429492073681684106974266732473283582251199684473394004038677069391278799297504466809439456560373351261561843732294201399342642485048861806520699838955215375938183164246905713902888830173868746004110336429406019431751890876414837974585857037931936009631605481447289893116786856562441832216311257042439806063785598342878372454731622929805073343996197573787090352073902245810345895873431467898909436762044613966967021911486188119609549831292025135993050932365492572744590585266402690739158346280929929978500499339008113747791946209747828024836255098012541106593813811665807502701513851726770557955311255012143102074491761548144980609065262303926782928259410970230923851333959833714917949253189276799418924788811164548907247060119625232347").unwrap();
        let m_tilde = mocks::mtilde();

        let m1_tilde = BigNumber::from_dec("67940925789970108743024738273926421512152745397724199848594503731042154269417576665420030681245389493783225644817826683796657351721363490290016166310023506339911751676800452438014771736117676826911321621579680668201191205819012441197794443970687648330757835198888257781967404396196813475280544039772512800509").unwrap();
        let m2_tilde = BigNumber::from_dec("6461691768834933403326572830814516653957231030793837560544354737855803497655300429843454445497126567767486684087006218691084619904526729989680526652503377438786587511370042964338").unwrap();
        let m2 = BigNumber::from_dec("52860447312636183767369476481903349046618423276302392993759146262753859184069").unwrap();

        PrimaryEqualInitProof {
            a_prime,
            t,
            e_tilde,
            e_prime,
            v_tilde,
            v_prime,
            m_tilde,
            m1_tilde,
            m2_tilde,
            m2
        }
    }

    pub fn primary_ge_init_proof() -> PrimaryPredicateGEInitProof {
        let c_list: Vec<BigNumber> = c_list();
        let tau_list: Vec<BigNumber> = tau_list();

        let mut u: HashMap<String, BigNumber> = HashMap::new();
        u.insert("0".to_string(), BigNumber::from_dec("3").unwrap());
        u.insert("1".to_string(), BigNumber::from_dec("1").unwrap());
        u.insert("2".to_string(), BigNumber::from_dec("0").unwrap());
        u.insert("3".to_string(), BigNumber::from_dec("0").unwrap());

        let mut u_tilde = HashMap::new();
        u_tilde.insert("3".to_string(), BigNumber::from_dec("6461691768834933403326572830814516653957231030793837560544354737855803497655300429843454445497126567767486684087006218691084619904526729989680526652503377438786587511370042964338").unwrap());
        u_tilde.insert("1".to_string(), BigNumber::from_dec("6461691768834933403326572830814516653957231030793837560544354737855803497655300429843454445497126567767486684087006218691084619904526729989680526652503377438786587511370042964338").unwrap());
        u_tilde.insert("2".to_string(), BigNumber::from_dec("6461691768834933403326572830814516653957231030793837560544354737855803497655300429843454445497126567767486684087006218691084619904526729989680526652503377438786587511370042964338").unwrap());
        u_tilde.insert("0".to_string(), BigNumber::from_dec("6461691768834933403326572830814516653957231030793837560544354737855803497655300429843454445497126567767486684087006218691084619904526729989680526652503377438786587511370042964338").unwrap());

        let mut r = HashMap::new();
        r.insert("3".to_string(), BigNumber::from_dec("1921424195886158938744777125021406748763985122590553448255822306242766229793715475428833504725487921105078008192433858897449555181018215580757557939320974389877538474522876366787859030586130885280724299566241892352485632499791646228580480458657305087762181033556428779333220803819945703716249441372790689501824842594015722727389764537806761583087605402039968357991056253519683582539703803574767702877615632257021995763302779502949501243649740921598491994352181379637769188829653918416991301420900374928589100515793950374255826572066003334385555085983157359122061582085202490537551988700484875690854200826784921400257387622318582276996322436").unwrap());
        r.insert("1".to_string(), BigNumber::from_dec("1921424195886158938744777125021406748763985122590553448255822306242766229793715475428833504725487921105078008192433858897449555181018215580757557939320974389877538474522876366787859030586130885280724299566241892352485632499791646228580480458657305087762181033556428779333220803819945703716249441372790689501824842594015722727389764537806761583087605402039968357991056253519683582539703803574767702877615632257021995763302779502949501243649740921598491994352181379637769188829653918416991301420900374928589100515793950374255826572066003334385555085983157359122061582085202490537551988700484875690854200826784921400257387622318582276996322436").unwrap());
        r.insert("2".to_string(), BigNumber::from_dec("1921424195886158938744777125021406748763985122590553448255822306242766229793715475428833504725487921105078008192433858897449555181018215580757557939320974389877538474522876366787859030586130885280724299566241892352485632499791646228580480458657305087762181033556428779333220803819945703716249441372790689501824842594015722727389764537806761583087605402039968357991056253519683582539703803574767702877615632257021995763302779502949501243649740921598491994352181379637769188829653918416991301420900374928589100515793950374255826572066003334385555085983157359122061582085202490537551988700484875690854200826784921400257387622318582276996322436").unwrap());
        r.insert("0".to_string(), BigNumber::from_dec("1921424195886158938744777125021406748763985122590553448255822306242766229793715475428833504725487921105078008192433858897449555181018215580757557939320974389877538474522876366787859030586130885280724299566241892352485632499791646228580480458657305087762181033556428779333220803819945703716249441372790689501824842594015722727389764537806761583087605402039968357991056253519683582539703803574767702877615632257021995763302779502949501243649740921598491994352181379637769188829653918416991301420900374928589100515793950374255826572066003334385555085983157359122061582085202490537551988700484875690854200826784921400257387622318582276996322436").unwrap());
        r.insert("DELTA".to_string(), BigNumber::from_dec("1921424195886158938744777125021406748763985122590553448255822306242766229793715475428833504725487921105078008192433858897449555181018215580757557939320974389877538474522876366787859030586130885280724299566241892352485632499791646228580480458657305087762181033556428779333220803819945703716249441372790689501824842594015722727389764537806761583087605402039968357991056253519683582539703803574767702877615632257021995763302779502949501243649740921598491994352181379637769188829653918416991301420900374928589100515793950374255826572066003334385555085983157359122061582085202490537551988700484875690854200826784921400257387622318582276996322436").unwrap());

        let mut r_tilde = HashMap::new();
        r_tilde.insert("3".to_string(), BigNumber::from_dec("7575191721496255329790454166600075461811327744716122725414003704363002865687003988444075479817517968742651133011723131465916075452356777073568785406106174349810313776328792235352103470770562831584011847").unwrap());
        r_tilde.insert("1".to_string(), BigNumber::from_dec("7575191721496255329790454166600075461811327744716122725414003704363002865687003988444075479817517968742651133011723131465916075452356777073568785406106174349810313776328792235352103470770562831584011847").unwrap());
        r_tilde.insert("2".to_string(), BigNumber::from_dec("7575191721496255329790454166600075461811327744716122725414003704363002865687003988444075479817517968742651133011723131465916075452356777073568785406106174349810313776328792235352103470770562831584011847").unwrap());
        r_tilde.insert("0".to_string(), BigNumber::from_dec("7575191721496255329790454166600075461811327744716122725414003704363002865687003988444075479817517968742651133011723131465916075452356777073568785406106174349810313776328792235352103470770562831584011847").unwrap());
        r_tilde.insert("DELTA".to_string(), BigNumber::from_dec("7575191721496255329790454166600075461811327744716122725414003704363002865687003988444075479817517968742651133011723131465916075452356777073568785406106174349810313776328792235352103470770562831584011847").unwrap());

        let alpha_tilde = BigNumber::from_dec("15019832071918025992746443764672619814038193111378331515587108416842661492145380306078894142589602719572721868876278167686578705125701790763532708415180504799241968357487349133908918935916667492626745934151420791943681376124817051308074507483664691464171654649868050938558535412658082031636255658721308264295197092495486870266555635348911182100181878388728256154149188718706253259396012667950509304959158288841789791483411208523521415447630365867367726300467842829858413745535144815825801952910447948288047749122728907853947789264574578039991615261320141035427325207080621563365816477359968627596441227854436137047681372373555472236147836722255880181214889123172703767379416198854131024048095499109158532300492176958443747616386425935907770015072924926418668194296922541290395990933578000312885508514814484100785527174742772860178035596639").unwrap();
        let predicate = predicate();

        let mut t = HashMap::new();
        t.insert("3".to_string(), BigNumber::from_dec("46369083086117629643055653975857627769028160828983987182078946658047913327657659075673217449651724551898727205835194812073207899212452294564444639346668484070129687160427147938076018605551830861026465851076491021338935906152700477977234743314769181602525430955162020248817746661022702546242365043781931307417744503802184994273068810023321000162105949048577491174537385619391992689890177380388187493777623608221690561227863928538947292434940859766215223694325554781311625439704847971277102325299579636232682943235572924328291095040633959587110788517670425708774447736335155403676598370782714048226320498065574125026899").unwrap());
        t.insert("1".to_string(), BigNumber::from_dec("42633794716405561166353758783443542082448925291459053109072523255543918476162700915813468558725428930654732720550388668689693688311928225615248227542838894861904877843723074396340940707779041622733024047596548590206852224857490474241304499513238502020545990648514598111266718428654653729661393150510227786297395151012680735494729670444556589448695350091598078767475426612902588875098609575406745197186551303270002056095805065181028711913238674710248448811408868490444106100385953490031500705851784934426334273103423243390196341490285527664863980694992161784435576660236953710046735477189662522764706620430688287285864").unwrap());
        t.insert("2".to_string(), BigNumber::from_dec("46369083086117629643055653975857627769028160828983987182078946658047913327657659075673217449651724551898727205835194812073207899212452294564444639346668484070129687160427147938076018605551830861026465851076491021338935906152700477977234743314769181602525430955162020248817746661022702546242365043781931307417744503802184994273068810023321000162105949048577491174537385619391992689890177380388187493777623608221690561227863928538947292434940859766215223694325554781311625439704847971277102325299579636232682943235572924328291095040633959587110788517670425708774447736335155403676598370782714048226320498065574125026899").unwrap());
        t.insert("0".to_string(), BigNumber::from_dec("78330570979325941798365644373115445702503890126796448033540676436952642712474355493362616083006349657268453144498828167557958002187631433688600374998507190955348534609331062289505584464470965930026066960445862271919137219085035331183489708020179104768806542397317724245476749638435898286962686099614654775075210180478240806960936772266501650713946075532415486293498432032415822169972407762416677793858709680700551196367079406811614109643837625095590323201355832120222436221544300974405069957610226245036804939616341080518318062198049430554737724174625842765640174768911551668897074696860939233144184997614684980589924").unwrap());
        t.insert("DELTA".to_string(), BigNumber::from_dec("55689486371095551191153293221620120399985911078762073609790094310886646953389020785947364735709221760939349576244277298015773664794725470336037959586509430339581241350326035321187900311380031369930812685369312069872023094452466688619635133201050270873513970497547720395196520621008569032923514500216567833262585947550373732948093781160931218148684610639834393439060745307992621402105096757255088629786888737281709324281552413987274960223110927132818654699339106642690418211294536451370321243108928564278387404368783012923356880461335644797776340191719071088431730682007888636922131293039620517120570619351490238276806").unwrap());

        PrimaryPredicateGEInitProof {
            c_list,
            tau_list,
            u,
            u_tilde,
            r,
            r_tilde,
            alpha_tilde,
            predicate,
            t
        }
    }

    pub fn c_list() -> Vec<BigNumber> {
        let mut c_list: Vec<BigNumber> = Vec::new();
        c_list.push(BigNumber::from_dec("78330570979325941798365644373115445702503890126796448033540676436952642712474355493362616083006349657268453144498828167557958002187631433688600374998507190955348534609331062289505584464470965930026066960445862271919137219085035331183489708020179104768806542397317724245476749638435898286962686099614654775075210180478240806960936772266501650713946075532415486293498432032415822169972407762416677793858709680700551196367079406811614109643837625095590323201355832120222436221544300974405069957610226245036804939616341080518318062198049430554737724174625842765640174768911551668897074696860939233144184997614684980589924").unwrap());
        c_list.push(BigNumber::from_dec("42633794716405561166353758783443542082448925291459053109072523255543918476162700915813468558725428930654732720550388668689693688311928225615248227542838894861904877843723074396340940707779041622733024047596548590206852224857490474241304499513238502020545990648514598111266718428654653729661393150510227786297395151012680735494729670444556589448695350091598078767475426612902588875098609575406745197186551303270002056095805065181028711913238674710248448811408868490444106100385953490031500705851784934426334273103423243390196341490285527664863980694992161784435576660236953710046735477189662522764706620430688287285864").unwrap());
        c_list.push(BigNumber::from_dec("46369083086117629643055653975857627769028160828983987182078946658047913327657659075673217449651724551898727205835194812073207899212452294564444639346668484070129687160427147938076018605551830861026465851076491021338935906152700477977234743314769181602525430955162020248817746661022702546242365043781931307417744503802184994273068810023321000162105949048577491174537385619391992689890177380388187493777623608221690561227863928538947292434940859766215223694325554781311625439704847971277102325299579636232682943235572924328291095040633959587110788517670425708774447736335155403676598370782714048226320498065574125026899").unwrap());
        c_list.push(BigNumber::from_dec("46369083086117629643055653975857627769028160828983987182078946658047913327657659075673217449651724551898727205835194812073207899212452294564444639346668484070129687160427147938076018605551830861026465851076491021338935906152700477977234743314769181602525430955162020248817746661022702546242365043781931307417744503802184994273068810023321000162105949048577491174537385619391992689890177380388187493777623608221690561227863928538947292434940859766215223694325554781311625439704847971277102325299579636232682943235572924328291095040633959587110788517670425708774447736335155403676598370782714048226320498065574125026899").unwrap());
        c_list.push(BigNumber::from_dec("55689486371095551191153293221620120399985911078762073609790094310886646953389020785947364735709221760939349576244277298015773664794725470336037959586509430339581241350326035321187900311380031369930812685369312069872023094452466688619635133201050270873513970497547720395196520621008569032923514500216567833262585947550373732948093781160931218148684610639834393439060745307992621402105096757255088629786888737281709324281552413987274960223110927132818654699339106642690418211294536451370321243108928564278387404368783012923356880461335644797776340191719071088431730682007888636922131293039620517120570619351490238276806").unwrap());
        c_list
    }

    pub fn tau_list() -> Vec<BigNumber> {
        let mut tau_list: Vec<BigNumber> = Vec::new();
        tau_list.push(BigNumber::from_dec("37691036678500088864090706889277344529085698202855318342609662324455534725777810174779988243614834740383002484042961779535438729512700925723800184769772855117653609397311580937440131814111009890073972276784593662470810723687676167680062717239972656425563430838236749325671702463390044920572001860955651242331741037260836613506653323682056706226370698422365916655999046380426509541586034749242827978969972239524676039139025602263974101808887008331192929679659076910995855665477952930199692854778469439162325030246066895851569630345729938981633504514117558420480144828304421708923356898912192737390539479512879411139535").unwrap());
        tau_list.push(BigNumber::from_dec("37691036678500088864090706889277344529085698202855318342609662324455534725777810174779988243614834740383002484042961779535438729512700925723800184769772855117653609397311580937440131814111009890073972276784593662470810723687676167680062717239972656425563430838236749325671702463390044920572001860955651242331741037260836613506653323682056706226370698422365916655999046380426509541586034749242827978969972239524676039139025602263974101808887008331192929679659076910995855665477952930199692854778469439162325030246066895851569630345729938981633504514117558420480144828304421708923356898912192737390539479512879411139535").unwrap());
        tau_list.push(BigNumber::from_dec("37691036678500088864090706889277344529085698202855318342609662324455534725777810174779988243614834740383002484042961779535438729512700925723800184769772855117653609397311580937440131814111009890073972276784593662470810723687676167680062717239972656425563430838236749325671702463390044920572001860955651242331741037260836613506653323682056706226370698422365916655999046380426509541586034749242827978969972239524676039139025602263974101808887008331192929679659076910995855665477952930199692854778469439162325030246066895851569630345729938981633504514117558420480144828304421708923356898912192737390539479512879411139535").unwrap());
        tau_list.push(BigNumber::from_dec("37691036678500088864090706889277344529085698202855318342609662324455534725777810174779988243614834740383002484042961779535438729512700925723800184769772855117653609397311580937440131814111009890073972276784593662470810723687676167680062717239972656425563430838236749325671702463390044920572001860955651242331741037260836613506653323682056706226370698422365916655999046380426509541586034749242827978969972239524676039139025602263974101808887008331192929679659076910995855665477952930199692854778469439162325030246066895851569630345729938981633504514117558420480144828304421708923356898912192737390539479512879411139535").unwrap());
        tau_list.push(BigNumber::from_dec("37691036678500088864090706889277344529085698202855318342609662324455534725777810174779988243614834740383002484042961779535438729512700925723800184769772855117653609397311580937440131814111009890073972276784593662470810723687676167680062717239972656425563430838236749325671702463390044920572001860955651242331741037260836613506653323682056706226370698422365916655999046380426509541586034749242827978969972239524676039139025602263974101808887008331192929679659076910995855665477952930199692854778469439162325030246066895851569630345729938981633504514117558420480144828304421708923356898912192737390539479512879411139535").unwrap());
        tau_list.push(BigNumber::from_dec("47065304866607958075946961264533928435933122536016679690080278659386698316132559908768761685743414728586341914305025339970537873714845915164843100776821561200343390749927996265246866447155790487554483555192805709960222015718787293872197230832464704800887153568636866026153126587657548580608446574507279965440247754859129693686186427399103313737110632413255017522482016458190003045641077338674019608347139399755470654452373975228190041980152120799403855480909173865431397307988238759767251890853580982844825639097363091181044515877489450972963624109587697097258041963985607958610791800500711857115582406526050626576194").unwrap());
        tau_list
    }

    pub fn mtilde() -> HashMap<String, BigNumber> {
        let mut mtilde = HashMap::new();
        mtilde.insert("height".to_string(), BigNumber::from_dec("6461691768834933403326572830814516653957231030793837560544354737855803497655300429843454445497126567767486684087006218691084619904526729989680526652503377438786587511370042964338").unwrap());
        mtilde.insert("age".to_string(), BigNumber::from_dec("6461691768834933403326572830814516653957231030793837560544354737855803497655300429843454445497126567767486684087006218691084619904526729989680526652503377438786587511370042964338").unwrap());
        mtilde.insert("sex".to_string(), BigNumber::from_dec("6461691768834933403326572830814516653957231030793837560544354737855803497655300429843454445497126567767486684087006218691084619904526729989680526652503377438786587511370042964338").unwrap());
        mtilde
    }

    pub fn eq_proof() -> PrimaryEqualProof {
        let mut revealed_attrs: HashMap<String, BigNumber> = HashMap::new();
        revealed_attrs.insert("name".to_string(), BigNumber::from_dec("1139481716457488690172217916278103335").unwrap());

        let mut m: HashMap<String, BigNumber> = HashMap::new();
        m.insert("age".to_string(), BigNumber::from_dec("6461691768834933403326572830814516653957231030793837560544354737855803497655300429843454445497126569555048377863338051254460267053606356944162460437192812434232786788496640641930").unwrap());
        m.insert("sex".to_string(), BigNumber::from_dec("6461691768834933403326573210330277861354501442113655769882988760097155977792459796092706040876245423440766971450670662675952825317632013652532469629317617583714945063045022245480").unwrap());
        m.insert("height".to_string(), BigNumber::from_dec("6461691768834933403326572830814516653957231030793837560544354737855803497655300429843454445497126578939747270189080172212182414586274398455192612806812346160325332993411278449288").unwrap());

        PrimaryEqualProof {
            revealed_attrs,
            a_prime: BigNumber::from_dec("55843178746788520435119921377390286268231906459093621159092786036045167470853237525750282569899268449257074522170415569390034716112368052014334531932423576902914762587842041027851281255714993168169397606683305211998762679155888831040910409096324726866578032630340975792746967149464040920396539558743125725962501042923116270042003617984521164987313533431008960902795014252204100220831622879715088891023369836472708006594040942575574164603258335948975612936260979474698634607068185834612498203540495530063764980572148330080073950973355083221837170619929706589683883912936747392721735783231955174948213742908010137510059").unwrap(),
            e: BigNumber::from_dec("162083298053730499878539868675621169436369451197643364049023367091432000132455800020257076120708238452276269567894398158050524769029842452").unwrap(),
            v: BigNumber::from_dec("241132863422049783305938575438970984968886174719777147285301544874444739988430486983952995820260097840839069440378394346753879604392851933586337358561654582254955422523682312898019724436529062720828313475600193232595180909715451462600156154893650696125774460890921102329846137078074382707757647774945043683720900258432094924802673704740906359826573881239743758685541361718016925837587696092410894740410057107458758557447620998360525272595382152876024857626077651578395225059761356087394593446798198952106185244345409492051630909148188459601119687896316694712657729049955088211383277523909796073178401612271233635772969741130113707687830672377452944696453914387935655490225348165302887058751092766205283395163137925794804058129811328946552494253968865890668669461930678189893633436518087402186509936569439729722305359388858552931085264558311600831146504998575898993873331466073087451895775323562065569650308485086554411659").unwrap(),
            m,
            m1: BigNumber::from_dec("67940925789970108743024738273926421512152745397724199848594503731042154269417576665420030681245389493783225644817826683796657351721363490290016166310023507717485270084329765530473051446860852464706042958744244302789614711475323377257526784176458583145354771172858130808467331945526902877938301707061812969839").unwrap(),
            m2: BigNumber::from_dec("6461691768834933403326576205504185652188833909247892684532892373128377494563032881387418065880893736940737234136484040150156890221922307233945402959479775946632391123924880274404").unwrap(),
        }
    }

    pub fn aggregated_proof() -> AggregatedProof {
        AggregatedProof {
            c_list: vec![vec![1, 186, 92, 249, 189, 141, 143, 77, 171, 208, 34, 140, 90, 244, 94, 183, 45, 154, 176, 130, 60, 178, 12, 91, 106, 61, 126, 148, 197, 182, 25, 153, 96, 174, 3, 165, 20, 89, 43, 231, 112, 217, 35, 100, 69, 135, 47, 144, 253, 40, 158, 137, 14, 165, 152, 246, 60, 170, 0, 228, 18, 85, 19, 117, 184, 191, 8, 222, 140, 135, 204, 99, 152, 191, 200, 124, 95, 124, 138, 86, 120, 75, 160, 110, 21, 36, 100, 161, 60, 215, 45, 138, 147, 21, 211, 241, 40, 25, 98, 21, 41, 160, 115, 84, 184, 92, 113, 251, 138, 182, 201, 12, 42, 35, 243, 28, 13, 195, 2, 30, 119, 253, 227, 15, 51, 237, 221, 14, 193, 142, 152, 182, 63, 150, 188, 87, 216, 26, 201, 10, 166, 26, 223, 177, 210, 90, 123, 241, 43, 125, 37, 94, 48, 89, 240, 144, 246, 246, 202, 224, 86, 207, 134, 211, 140, 154, 77, 45, 168, 99, 192, 41, 142, 42, 106, 165, 64, 130, 26, 255, 247, 56, 250, 156, 193, 209, 139, 4, 234, 227, 138, 199, 99, 151, 3, 89, 46, 142, 137, 27, 152, 205, 147, 136, 121, 32, 126, 71, 112, 40, 0, 236, 30, 62, 12, 66, 74, 177, 19, 170, 170, 14, 149, 90, 43, 199, 68, 15, 239, 213, 131, 33, 112, 117, 13, 101, 181, 164, 202, 58, 143, 46, 105, 23, 171, 178, 36, 198, 189, 220, 128, 247, 59, 129, 189, 224, 171],
                         vec![2, 108, 127, 101, 174, 218, 32, 134, 244, 38, 234, 207, 183, 66, 169, 248, 220, 152, 219, 224, 147, 85, 180, 138, 119, 9, 112, 56, 171, 119, 32, 85, 150, 21, 32, 246, 205, 201, 127, 46, 230, 100, 227, 32, 121, 190, 24, 173, 28, 86, 154, 44, 66, 119, 101, 162, 138, 185, 201, 243, 172, 229, 25, 147, 210, 51, 172, 170, 113, 11, 245, 227, 33, 4, 197, 168, 253, 19, 136, 59, 158, 255, 53, 184, 168, 158, 46, 232, 119, 185, 114, 41, 17, 179, 201, 109, 92, 53, 238, 69, 40, 13, 2, 122, 179, 99, 68, 189, 76, 41, 105, 70, 85, 127, 150, 192, 111, 167, 53, 48, 221, 242, 243, 164, 202, 56, 243, 146, 104, 122, 12, 173, 136, 61, 169, 225, 79, 41, 180, 155, 198, 21, 192, 140, 223, 100, 207, 167, 50, 100, 17, 2, 102, 161, 47, 187, 96, 210, 156, 24, 214, 179, 43, 158, 9, 191, 186, 75, 40, 216, 47, 145, 104, 23, 8, 119, 90, 69, 104, 83, 183, 200, 85, 140, 134, 172, 12, 251, 73, 172, 157, 33, 100, 226, 180, 51, 102, 151, 36, 253, 149, 15, 97, 191, 210, 246, 28, 120, 161, 126, 51, 99, 181, 225, 54, 24, 131, 91, 178, 164, 116, 32, 67, 30, 181, 227, 245, 241, 172, 153, 113, 14, 127, 6, 98, 199, 250, 43, 119, 146, 160, 105, 138, 190, 162, 9, 230, 81, 116, 42, 31, 84, 160, 67, 219, 53, 100],
                         vec![1, 81, 185, 134, 123, 21, 18, 221, 49, 172, 39, 239, 236, 207, 16, 143, 240, 173, 88, 153, 7, 162, 166, 60, 151, 232, 163, 185, 151, 27, 178, 120, 14, 12, 201, 119, 144, 135, 130, 203, 231, 119, 46, 249, 128, 137, 136, 243, 91, 240, 120, 169, 203, 72, 35, 17, 151, 39, 246, 124, 44, 135, 141, 132, 178, 89, 195, 178, 253, 153, 216, 48, 226, 115, 1, 36, 137, 191, 159, 106, 192, 193, 254, 50, 97, 50, 204, 141, 202, 207, 8, 168, 100, 200, 247, 209, 198, 213, 58, 213, 202, 226, 82, 214, 206, 99, 143, 121, 91, 80, 19, 251, 59, 64, 79, 221, 234, 219, 244, 174, 44, 100, 141, 29, 163, 221, 175, 180, 131, 141, 42, 209, 0, 36, 199, 9, 10, 134, 93, 103, 96, 7, 11, 197, 228, 166, 132, 242, 31, 233, 228, 117, 242, 242, 64, 5, 21, 252, 184, 181, 124, 66, 168, 126, 165, 69, 30, 218, 112, 124, 134, 57, 143, 200, 9, 0, 71, 72, 251, 216, 5, 68, 126, 168, 209, 162, 147, 106, 245, 106, 240, 86, 56, 96, 124, 242, 119, 141, 132, 145, 104, 68, 224, 33, 61, 1, 16, 242, 210, 43, 56, 209, 209, 128, 200, 208, 54, 249, 111, 136, 246, 154, 105, 73, 64, 139, 81, 85, 177, 174, 214, 250, 59, 161, 159, 174, 38, 94, 195, 191, 120, 33, 69, 179, 235, 20, 106, 133, 209, 118, 61, 159, 242, 0, 101, 98, 104],
                         vec![1, 111, 80, 91, 53, 214, 139, 10, 197, 79, 134, 183, 50, 233, 244, 130, 80, 173, 167, 5, 130, 151, 183, 162, 97, 134, 246, 146, 37, 151, 103, 45, 68, 33, 204, 18, 157, 21, 98, 230, 225, 30, 162, 172, 75, 159, 115, 94, 72, 113, 153, 155, 117, 233, 95, 251, 29, 1, 149, 38, 117, 63, 112, 213, 48, 29, 3, 131, 238, 120, 48, 141, 105, 31, 127, 51, 176, 32, 203, 191, 155, 159, 91, 29, 87, 223, 30, 92, 146, 250, 182, 181, 155, 67, 253, 33, 165, 142, 195, 146, 180, 221, 83, 62, 46, 74, 29, 83, 175, 218, 132, 93, 42, 93, 105, 173, 189, 254, 193, 230, 113, 39, 45, 137, 143, 124, 190, 42, 19, 77, 13, 220, 137, 202, 128, 170, 10, 22, 37, 177, 200, 186, 3, 73, 171, 232, 81, 144, 36, 46, 70, 237, 208, 26, 84, 26, 141, 19, 37, 200, 83, 60, 27, 175, 96, 233, 246, 144, 137, 178, 140, 213, 13, 36, 137, 82, 107, 0, 239, 192, 187, 126, 20, 205, 40, 203, 33, 238, 88, 121, 132, 31, 87, 91, 65, 207, 144, 15, 249, 66, 58, 98, 64, 61, 236, 103, 203, 207, 20, 205, 48, 202, 247, 22, 248, 197, 188, 21, 178, 187, 193, 152, 164, 247, 53, 15, 33, 170, 145, 3, 213, 63, 205, 55, 158, 170, 62, 157, 207, 162, 117, 157, 215, 125, 94, 77, 251, 251, 25, 209, 207, 119, 16, 186, 210, 190, 83],
                         vec![1, 111, 80, 91, 53, 214, 139, 10, 197, 79, 134, 183, 50, 233, 244, 130, 80, 173, 167, 5, 130, 151, 183, 162, 97, 134, 246, 146, 37, 151, 103, 45, 68, 33, 204, 18, 157, 21, 98, 230, 225, 30, 162, 172, 75, 159, 115, 94, 72, 113, 153, 155, 117, 233, 95, 251, 29, 1, 149, 38, 117, 63, 112, 213, 48, 29, 3, 131, 238, 120, 48, 141, 105, 31, 127, 51, 176, 32, 203, 191, 155, 159, 91, 29, 87, 223, 30, 92, 146, 250, 182, 181, 155, 67, 253, 33, 165, 142, 195, 146, 180, 221, 83, 62, 46, 74, 29, 83, 175, 218, 132, 93, 42, 93, 105, 173, 189, 254, 193, 230, 113, 39, 45, 137, 143, 124, 190, 42, 19, 77, 13, 220, 137, 202, 128, 170, 10, 22, 37, 177, 200, 186, 3, 73, 171, 232, 81, 144, 36, 46, 70, 237, 208, 26, 84, 26, 141, 19, 37, 200, 83, 60, 27, 175, 96, 233, 246, 144, 137, 178, 140, 213, 13, 36, 137, 82, 107, 0, 239, 192, 187, 126, 20, 205, 40, 203, 33, 238, 88, 121, 132, 31, 87, 91, 65, 207, 144, 15, 249, 66, 58, 98, 64, 61, 236, 103, 203, 207, 20, 205, 48, 202, 247, 22, 248, 197, 188, 21, 178, 187, 193, 152, 164, 247, 53, 15, 33, 170, 145, 3, 213, 63, 205, 55, 158, 170, 62, 157, 207, 162, 117, 157, 215, 125, 94, 77, 251, 251, 25, 209, 207, 119, 16, 186, 210, 190, 83],
                         vec![1, 185, 37, 77, 23, 245, 214, 239, 127, 18, 101, 63, 229, 201, 171, 193, 32, 182, 124, 45, 15, 127, 58, 172, 226, 30, 246, 70, 33, 19, 117, 183, 29, 157, 209, 237, 41, 58, 208, 4, 105, 26, 73, 26, 69, 72, 21, 78, 106, 28, 72, 117, 102, 144, 199, 148, 3, 98, 81, 251, 246, 106, 50, 235, 129, 14, 186, 108, 216, 29, 41, 207, 233, 7, 179, 86, 224, 230, 187, 138, 125, 62, 68, 31, 66, 147, 205, 93, 100, 9, 134, 225, 210, 57, 36, 71, 134, 26, 179, 85, 37, 194, 32, 137, 91, 4, 91, 214, 220, 134, 173, 148, 14, 95, 209, 232, 79, 87, 12, 180, 217, 148, 240, 242, 190, 36, 229, 189, 16, 208, 75, 176, 153, 239, 212, 255, 45, 42, 250, 234, 139, 40, 104, 74, 21, 30, 184, 221, 126, 185, 23, 69, 114, 104, 249, 242, 248, 210, 97, 100, 141, 61, 176, 93, 200, 148, 152, 138, 31, 66, 99, 61, 237, 210, 42, 205, 60, 241, 92, 247, 1, 146, 203, 116, 237, 0, 171, 235, 250, 128, 74, 56, 223, 65, 189, 176, 91, 243, 174, 2, 111, 216, 233, 227, 28, 22, 41, 102, 225, 1, 21, 156, 212, 16, 243, 9, 94, 61, 246, 153, 193, 243, 188, 187, 154, 109, 168, 36, 89, 48, 236, 113, 74, 179, 158, 103, 51, 38, 15, 148, 18, 89, 218, 144, 71, 198, 8, 144, 104, 135, 160, 224, 98, 243, 106, 228, 198]],
            c_hash: BigNumber::from_dec("63841489063440422591549130255324272391231497635167479821265935688468807059914").unwrap()
        }
    }

    pub fn ge_proof() -> PrimaryPredicateGEProof {
        let mut m: HashMap<String, BigNumber> = HashMap::new();
        m.insert("age".to_string(), BigNumber::from_dec("6461691768834933403326572830814516653957231030793837560544354737855803497655300429843454445497126569555048377863338051254460267053606356944162460437192812434232786788496640641930").unwrap());
        m.insert("sex".to_string(), BigNumber::from_dec("6461691768834933403326573210330277861354501442113655769882988760097155977792459796092706040876245423440766971450670662675952825317632013652532469629317617583714945063045022245480").unwrap());
        m.insert("height".to_string(), BigNumber::from_dec("6461691768834933403326572830814516653957231030793837560544354737855803497655300429843454445497126578939747270189080172212182414586274398455192612806812346160325332993411278449288").unwrap());

        let mut u: HashMap<String, BigNumber> = HashMap::new();
        u.insert("2".to_string(), BigNumber::from_dec("6461691768834933403326572830814516653957231030793837560544354737855803497655300429843454445497126567767486684087006218691084619904526729989680526652503377438786587511370042964338").unwrap());
        u.insert("1".to_string(), BigNumber::from_dec("6461691768834933403326572830814516653957231030793837560544354737855803497655300429843454445497126567831328173150446641282633750159851002380912024287670857260052523199838850024252").unwrap());
        u.insert("0".to_string(), BigNumber::from_dec("6461691768834933403326572830814516653957231030793837560544354737855803497655300429843454445497126567959011151277327486465732010670499547163375019558005816902584394576776464144080").unwrap());
        u.insert("3".to_string(), BigNumber::from_dec("6461691768834933403326572830814516653957231030793837560544354737855803497655300429843454445497126567767486684087006218691084619904526729989680526652503377438786587511370042964338").unwrap());

        let mut r: HashMap<String, BigNumber> = HashMap::new();
        r.insert("2".to_string(), BigNumber::from_dec("122666581787896024104771761595539708848783314985870238259074669824520091098683817237172519182829174751114708491011709191270412318634809532273931666000301987869809614370778701672920770190235911538453236520585124998634470107126877826855765108565024357739461476219090897270520451817930736172663543943052827769367981507788289923500996293391654370634807890778790076616041326007628068206880269267272777192271905638118708385050200412890391080370252730064261452554932992620443959769478748678597670501698531981378757093642774169056547668193201752061644097178572361915153806621540894628974958162220867331621188215651633938457631228059207968660364669634554543579944958864314375144914088839439106378569969245085620007043098442351").unwrap());
        r.insert("1".to_string(), BigNumber::from_dec("122666581787896024104771761595539708848783314985870238259074669824520091098683817237172519182829174751114708491011709191270412318634809532273931666000301987869809614370778701672920770190235911538453236520585124998634470107126877826855765108565024357739461476219090897270520451817930736172663543943052827769367981507788289923500996293391654370634807890778790076616041326007628068206880269267272777192271905638118708385050200412890391080370252730064261452554932992620443959769478748678597670501698531981378757093642774169056547668193201752061644097178572361915153806621540894628974958162220867331621188215651633938457631228059207968660364669634554543579944958864314375144914088839439106378569969245085620007043098442351").unwrap());
        r.insert("0".to_string(), BigNumber::from_dec("122666581787896024104771761595539708848783314985870238259074669824520091098683817237172519182829174751114708491011709191270412318634809532273931666000301987869809614370778701672920770190235911538453236520585124998634470107126877826855765108565024357739461476219090897270520451817930736172663543943052827769367981507788289923500996293391654370634807890778790076616041326007628068206880269267272777192271905638118708385050200412890391080370252730064261452554932992620443959769478748678597670501698531981378757093642774169056547668193201752061644097178572361915153806621540894628974958162220867331621188215651633938457631228059207968660364669634554543579944958864314375144914088839439106378569969245085620007043098442351").unwrap());
        r.insert("3".to_string(), BigNumber::from_dec("122666581787896024104771761595539708848783314985870238259074669824520091098683817237172519182829174751114708491011709191270412318634809532273931666000301987869809614370778701672920770190235911538453236520585124998634470107126877826855765108565024357739461476219090897270520451817930736172663543943052827769367981507788289923500996293391654370634807890778790076616041326007628068206880269267272777192271905638118708385050200412890391080370252730064261452554932992620443959769478748678597670501698531981378757093642774169056547668193201752061644097178572361915153806621540894628974958162220867331621188215651633938457631228059207968660364669634554543579944958864314375144914088839439106378569969245085620007043098442351").unwrap());
        r.insert("DELTA".to_string(), BigNumber::from_dec("122666581787896024104771761595539708848783314985870238259074669824520091098683817237172519182829174751114708491011709191270412318634809532273931666000301987869809614370778701672920770190235911538453236520585124998634470107126877826855765108565024357739461476219090897270520451817930736172663543943052827769367981507788289923500996293391654370634807890778790076616041326007628068206880269267272777192271905638118708385050200412890391080370252730064261452554932992620443959769478748678597670501698531981378757093642774169056547668193201752061644097178572361915153806621540894628974958162220867331621188215651633938457631228059207968660364669634554543579944958864314375144914088839439106378569969245085620007043098442351").unwrap());

        let mut t: HashMap<String, BigNumber> = HashMap::new();
        t.insert("2".to_string(), BigNumber::from_dec("46369083086117629643055653975857627769028160828983987182078946658047913327657659075673217449651724551898727205835194812073207899212452294564444639346668484070129687160427147938076018605551830861026465851076491021338935906152700477977234743314769181602525430955162020248817746661022702546242365043781931307417744503802184994273068810023321000162105949048577491174537385619391992689890177380388187493777623608221690561227863928538947292434940859766215223694325554781311625439704847971277102325299579636232682943235572924328291095040633959587110788517670425708774447736335155403676598370782714048226320498065574125026899").unwrap());
        t.insert("1".to_string(), BigNumber::from_dec("42633794716405561166353758783443542082448925291459053109072523255543918476162700915813468558725428930654732720550388668689693688311928225615248227542838894861904877843723074396340940707779041622733024047596548590206852224857490474241304499513238502020545990648514598111266718428654653729661393150510227786297395151012680735494729670444556589448695350091598078767475426612902588875098609575406745197186551303270002056095805065181028711913238674710248448811408868490444106100385953490031500705851784934426334273103423243390196341490285527664863980694992161784435576660236953710046735477189662522764706620430688287285864").unwrap());
        t.insert("0".to_string(), BigNumber::from_dec("78330570979325941798365644373115445702503890126796448033540676436952642712474355493362616083006349657268453144498828167557958002187631433688600374998507190955348534609331062289505584464470965930026066960445862271919137219085035331183489708020179104768806542397317724245476749638435898286962686099614654775075210180478240806960936772266501650713946075532415486293498432032415822169972407762416677793858709680700551196367079406811614109643837625095590323201355832120222436221544300974405069957610226245036804939616341080518318062198049430554737724174625842765640174768911551668897074696860939233144184997614684980589924").unwrap());
        t.insert("3".to_string(), BigNumber::from_dec("46369083086117629643055653975857627769028160828983987182078946658047913327657659075673217449651724551898727205835194812073207899212452294564444639346668484070129687160427147938076018605551830861026465851076491021338935906152700477977234743314769181602525430955162020248817746661022702546242365043781931307417744503802184994273068810023321000162105949048577491174537385619391992689890177380388187493777623608221690561227863928538947292434940859766215223694325554781311625439704847971277102325299579636232682943235572924328291095040633959587110788517670425708774447736335155403676598370782714048226320498065574125026899").unwrap());
        t.insert("DELTA".to_string(), BigNumber::from_dec("55689486371095551191153293221620120399985911078762073609790094310886646953389020785947364735709221760939349576244277298015773664794725470336037959586509430339581241350326035321187900311380031369930812685369312069872023094452466688619635133201050270873513970497547720395196520621008569032923514500216567833262585947550373732948093781160931218148684610639834393439060745307992621402105096757255088629786888737281709324281552413987274960223110927132818654699339106642690418211294536451370321243108928564278387404368783012923356880461335644797776340191719071088431730682007888636922131293039620517120570619351490238276806").unwrap());

        PrimaryPredicateGEProof {
            u,
            r,
            mj: BigNumber::from_dec("6461691768834933403326572830814516653957231030793837560544354737855803497655300429843454445497126569555048377863338051254460267053606356944162460437192812434232786788496640641930").unwrap(),
            alpha: BigNumber::from_dec("15019832071918025992746443764672619814038193111378331515587108416842661492145380306078894142589602719572721868876278167686210705380338102691218393130393885672695618412529738419131694926443107219330694482439903234395193851871472925835039379909853454508267226053046255940557629449048653188523919553702545953724489357880127160704800260353007771778801244908160960828454115645487868830738739976138947949505366080323799159654252725215417470924265496096864737420292879717953990073198774585977677974887563743667406941320910576277132072350218452884841014022648967794316567016887837205701017499498636748288004981818643125542585776429419200955219536940661401665401273238350271276070084547091903752551649057233346746822426635975545515195870976674441104284294336189831971933619615980881781820696853193401192672937826151341781675749898224527543492305127").unwrap(),
            t,
            predicate: predicate()
        }
    }

    pub fn primary_proof() -> PrimaryProof {
        PrimaryProof {
            eq_proof: eq_proof(),
            ge_proofs: vec![ge_proof()]
        }
    }

    pub fn sub_proof_request() -> SubProofRequest {
        SubProofRequestBuilder::new().unwrap()
            .add_revealed_attr("name").unwrap()
            .add_predicate(&predicate()).unwrap()
            .finalize().unwrap()
    }

    pub fn revealed_attrs() -> HashSet<String> {
        HashSet::from_iter(vec!["name".to_owned()].into_iter())
    }

    pub fn unrevealed_attrs() -> HashSet<String> {
        HashSet::from_iter(vec!["height".to_owned(), "age".to_owned(), "sex".to_owned()])
    }

    pub fn claim_revealed_attributes_values() -> ClaimValues {
        ClaimValuesBuilder::new().unwrap()
            .add_value("name", "1139481716457488690172217916278103335").unwrap()
            .finalize().unwrap()
    }

    pub fn predicate() -> Predicate {
        Predicate {
            attr_name: "age".to_owned(),
            p_type: PredicateType::GE,
            value: 18
        }
    }
}