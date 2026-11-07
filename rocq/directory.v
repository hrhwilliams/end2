Require Import List.
Import ListNotations.

Section KeyDirectory.

Variables Key UserName UserIdentifier Device Sig : Type.
Variable signed_by : Key -> Sig -> Prop.

Inductive Entry : Type :=
  (* user's first device. inductive base case with no signature *)
  | D_root : UserIdentifier -> Device -> Key -> Entry
  (* device with a signature from another device *)
  | D_signed : UserIdentifier -> Device -> Key -> Key -> Sig -> Entry
  (* updated device that invalidates a previous device *)
  | D_updated : UserIdentifier -> Device -> Key -> Key -> Sig -> Entry.

(* a key directory is a list of entries *)
Definition Directory := list Entry.


(* (u, d, k) for a particular k is in the directory *)
Inductive InDir (u : UserIdentifier) (d : Device) (k : Key) : Directory -> Prop :=
  | In_root : forall D, InDir u d k (D_root u d k :: D)
  | In_signed : forall D ki s, InDir u d k (D_signed u d k ki s :: D)
  | In_updated : forall D kp s, InDir u d k (D_updated u d k kp s :: D)
  (* destructure an entity from the directory *)
  | In_destruct  : forall e D, InDir u d k D -> InDir u d k (e :: D).

(* shorthands for saying a particular user or device is in the
   directory *)

Definition DeviceExists (u : UserIdentifier) (d : Device) (D : Directory) : Prop :=
  exists k, InDir u d k D.

Definition UserExists (u : UserIdentifier) (D : Directory) : Prop :=
  exists d k, InDir u d k D.


(* rules for going from Lists:In to InDir *)

Lemma in_root_to_indir :
  forall u d k D, In (D_root u d k) D -> InDir u d k D.
Proof.
  induction D as [| e D' IH]; intros H; [inversion H |].
  destruct H as [E | H]; [subst; apply In_root | apply In_destruct;
    auto].
Qed.

Lemma in_signed_to_indir :
  forall u d k ki s D, In (D_signed u d k ki s) D -> InDir u d k D.
Proof.
  induction D as [| e D' IH]; intros H; [inversion H |].
  destruct H as [E | H]; [subst; apply In_signed | apply In_destruct;
    eauto].
Qed.

Lemma in_updated_to_indir :
  forall u d k kp s D, In (D_updated u d k kp s) D -> InDir u d k D.
Proof.
  induction D as [| e D' IH]; intros H; [inversion H |].
  destruct H as [E | H]; [subst; apply In_updated | apply In_destruct;
    eauto].
Qed.

Hint Resolve in_root_to_indir in_signed_to_indir in_updated_to_indir
  : dir.


(* a directory is valid iff every entry was added under the following
   preconditions *)
Inductive DirIsValid : Directory -> Prop :=
  | V_nil : DirIsValid nil
  | V_root : forall u d k D,
      DirIsValid D ->
      ~ UserExists u D ->
      DirIsValid (D_root u d k :: D)
  | V_new : forall u d k ki s D d_i,
      DirIsValid D ->
      ~ DeviceExists u d D ->
      InDir u d_i ki D ->
      signed_by ki s ->
      DirIsValid (D_signed u d k ki s :: D)
  | V_update : forall u d k kp s D,
      DirIsValid D ->
      InDir u d kp D ->
      signed_by kp s ->
      DirIsValid (D_updated u d k kp s :: D).

(* trace from a device to the user's root device via signatures *)
Inductive TracesToRoot (u : UserIdentifier) : Device -> Key -> Directory -> Prop :=
  | TR_root : forall d k D,
      In (D_root u d k) D ->
      TracesToRoot u d k D
  | TR_new : forall d k ki s D d_i,
      In (D_signed u d k ki s) D ->
      signed_by ki s ->
      TracesToRoot u d_i ki D ->
      TracesToRoot u d k D
  | TR_rot : forall d k kp s D,
      In (D_updated u d k kp s) D ->
      signed_by kp s ->
      TracesToRoot u d kp D ->
      TracesToRoot u d k D.

Lemma trace_implies_indir :
  forall u d k D, TracesToRoot u d k D -> InDir u d k D.
Proof.
  intros u d k D Htr.
  induction Htr; eauto with dir.
Qed.

(* destructure entity from directory via trace *)
Lemma traces_destruct :
  forall u d k D e, TracesToRoot u d k D -> TracesToRoot u d k (e :: D).
Proof.
  intros u d k D e H.
  induction H.
  - apply TR_root. right. assumption.
  - eapply TR_new.
    + right. eassumption.
    + assumption.
    + exact IHTracesToRoot.
  - eapply TR_rot.
    + right. eassumption.
    + assumption.
    + assumption.
Qed.

(* a key k is authentic if a root key k_r exists,
   k can be traced back to k_r, and k_r is unique. *)
Definition KeyAuthentic (D : Directory) (u : UserIdentifier) (d : Device)
    (k : Key) : Prop :=
  exists d_r k_r,
    In (D_root u d_r k_r) D /\
    TracesToRoot u d k D /\
    (forall d' k', In (D_root u d' k') D -> d' = d_r /\ k' = k_r).

(* user's root device is unique *)
Lemma root_unique :
  forall D, DirIsValid D ->
  forall u d1 k1 d2 k2,
    In (D_root u d1 k1) D -> In (D_root u d2 k2) D ->
    d1 = d2 /\ k1 = k2.
Proof.
  induction D as [| e D IH]; intros HV u d1 k1 d2 k2 H1 H2.
  - inversion H1.
  - inversion HV; subst.
    + destruct H1 as [E1 | H1]; destruct H2 as [E2 | H2].
      * inversion E1; inversion E2; subst; split; reflexivity.
      * inversion E1; subst.
        exfalso. apply H4. exists d2, k2.
        clear -H2. induction D as [| e' D' IH'].
        -- inversion H2.
        -- destruct H2 as [E | H2].
           ++ subst. apply In_root.
           ++ apply In_destruct. apply IH'. assumption.
      * inversion E2; subst.
        exfalso. apply H4. exists d1, k1.
        clear -H1. induction D as [| e' D' IH'].
        -- inversion H1.
        -- destruct H1 as [E | H1].
           ++ subst. apply In_root.
           ++ apply In_destruct. apply IH'. assumption.
      * apply IH with u; assumption.
    + destruct H1 as [E1 | H1]; [discriminate |].
      destruct H2 as [E2 | H2]; [discriminate |].
      apply IH with u; assumption.
    + destruct H1 as [E1 | H1]; [discriminate |].
      destruct H2 as [E2 | H2]; [discriminate |].
      apply IH with u; assumption.
Qed.

(* if a key k has a trace to a root, that means a root key exists *)
Lemma trace_has_root :
  forall u d k D, TracesToRoot u d k D -> exists d_r k_r, In
    (D_root u d_r k_r) D.
Proof.
  intros u d k D Htr.
  induction Htr; eauto.
Qed.

(* for every user u with device keys (d k) in the directory, those keys
   trace back to one and only one root key (d_r, k_r) *)
Theorem chain_of_trust :
  forall D, DirIsValid D ->
  forall u d k, InDir u d k D ->
    exists d_r k_r,
      In (D_root u d_r k_r) D /\
      TracesToRoot u d k D /\
      (forall d' k', In (D_root u d' k') D -> d' = d_r /\ k' = k_r).
Proof.
  induction D as [| e D IH]; intros HV u d k Hin.
  - inversion Hin.
  - assert (TracesToRoot u d k (e :: D)) as Htr.
    { inversion HV; subst.
      + inversion Hin; subst.
        * apply TR_root. left. reflexivity.
        * apply traces_destruct.
          destruct (IH H1 u d k H0) as [d_r [k_r [_ [Htr _]]]].
          assumption.
      + inversion Hin; subst.
        * eapply TR_new.
          -- left. reflexivity.
          -- eassumption.
          -- apply traces_destruct.
             destruct (IH H1 u0 d_i ki H3) as [d_r [k_r [_ [Htr _]]]].
             exact Htr.
        * apply traces_destruct.
          destruct (IH H1 u d k H0) as [d_r [k_r [_ [Htr _]]]].
          assumption.
      + inversion Hin; subst.
        * eapply TR_rot.
          -- left. reflexivity.
          -- assumption.
          -- apply traces_destruct.
             destruct (IH H1 u0 d0 kp H2) as [d_r [k_r [_ [Htr _]]]].
             assumption.
        * apply traces_destruct.
          destruct (IH H1 u d k H0) as [d_r [k_r [_ [Htr _]]]].
          assumption. }
    destruct (trace_has_root _ _ _ _ Htr) as [d_r [k_r Hroot]].
    exists d_r, k_r.
    split; [assumption | split; [assumption |]].
    intros d' k' Hroot'.
    eapply root_unique; eassumption.
Qed.

(* small step semantics: updating the state of the directory should maintain
   its validity, as well as the authenticity of all keys within it *)
Inductive Op : Type :=
  | Op_Register : UserIdentifier -> Device -> Key -> Op
  | Op_AddDevice : UserIdentifier -> Device -> Key -> Key -> Sig -> Op
  | Op_Update : UserIdentifier -> Device -> Key -> Key -> Sig -> Op.

Inductive Step : Directory -> Op -> Directory -> Prop :=
  | Step_Register : forall D u d k,
      DirIsValid D ->
      ~ UserExists u D ->
      Step D (Op_Register u d k) (D_root u d k :: D)
  | Step_AddDevice : forall D u d k ki s d_i,
      DirIsValid D ->
      ~ DeviceExists u d D ->
      InDir u d_i ki D ->
      signed_by ki s ->
      Step D (Op_AddDevice u d k ki s) (D_signed u d k ki s :: D)
  | Step_Update : forall D u d k kp s,
      DirIsValid D ->
      InDir u d kp D ->
      signed_by kp s ->
      Step D (Op_Update u d k kp s) (D_updated u d k kp s :: D).

Theorem step_preserves_validity :
  forall D op D',
    Step D op D' ->
    DirIsValid D'.
Proof.
  intros D op D' Hstep.
  inversion Hstep; subst.
  - apply V_root; assumption.
  - eapply V_new; eassumption.
  - eapply V_update; eassumption.
Qed.

Theorem step_preserves_authenticity :
  forall D op D' u d k,
    Step D op D' ->
    KeyAuthentic D u d k ->
    KeyAuthentic D' u d k.
Proof.
  intros D op D' u d k Hstep Hauth.
  destruct Hauth as [d_r [k_r [Hroot [Htr Huniq]]]].
  pose proof (step_preserves_validity _ _ _ Hstep) as HV'.
  inversion Hstep; subst.
  all: exists d_r, k_r;
       split; [right; assumption |];
       split; [apply traces_destruct; assumption |];
       intros d' k' Hroot';
       eapply root_unique; [exact HV' | exact Hroot' | right; exact Hroot].
Qed.

(* if a key is in the directory, it must trace back to some root,
   and vice-versa *)
Corollary membership_iff_trace :
  forall D, DirIsValid D ->
  forall u d k, InDir u d k D <-> TracesToRoot u d k D.
Proof.
  intros D HV u d k. split.
  - intros Hin.
    destruct (chain_of_trust D HV u d k Hin) as [_ [_ [_ [Htr _]]]].
    exact Htr.
  - apply trace_implies_indir.
Qed.

(* the service cannot distribute malicious keys not tied to a user's root
   keys *)
Corollary distributed_keys_authentic :
  forall D u_A u_B d_A d_B k_A k_B,
    (* in a key directory where u_A and u_B have the device keys k_A
       and k_B *)
    DirIsValid D ->
    InDir u_A d_A k_A D ->
    InDir u_B d_B k_B D ->
    (* both users have a root key which k_A and k_B trace back to *)
    KeyAuthentic D u_A d_A k_A /\ KeyAuthentic D u_B d_B k_B.
Proof.
  intros D u_A u_B d_A d_B k_A k_B HV HinA HinB.
  split; apply chain_of_trust; assumption.
Qed.

(* service cannot insert a new root device key *)
Corollary no_hidden_root :
  (* if the directory is valid and the user exists in it, *)
  forall D u d d_m k k_m,
    DirIsValid D ->
    InDir u d k D ->
    (* then inserting a new root under that user invalidates
       the directory. *)
    ~ DirIsValid (D_root u d_m k_m :: D).
Proof.
  intros D u d d_m k k_m HV Hin Hbad.
  inversion Hbad; subst.
  apply H4. exists d, k. assumption.
Qed.

(* a signed key can only be included in the directory if it has a
   valid signature from a key already belonging to that user *)
Corollary no_hidden_keys :
  forall D u d k ki s,
    DirIsValid (D_signed u d k ki s :: D) ->
    (exists d_i, InDir u d_i ki D) /\ signed_by ki s.
Proof.
  intros D u d_m k_m ki s HV.
  inversion HV; subst. split; [exists d_i; assumption | assumption].
Qed.

(* cryptographic assumption that keys are unique in a directory. *)
Hypothesis key_owner_unique :
  forall D u u' d d' k,
    DirIsValid D ->
    InDir u d k D ->
    InDir u' d' k D ->
    u = u' /\ d = d'.

(* k_A and k_B trace to their owners' roots, and neither key can
   exist in the directory under any other user *)
Corollary no_pitm :
  forall D u_A u_B d_A d_B k_A k_B,
    DirIsValid D ->
    InDir u_A d_A k_A D ->
    InDir u_B d_B k_B D ->
    KeyAuthentic D u_A d_A k_A /\ KeyAuthentic D u_B d_B k_B /\
    (* k_A is not also in the directory under any user other than u_A *)
    (forall u' d' , InDir u' d' k_A D -> u' = u_A) /\
    (forall u' d' , InDir u' d' k_B D -> u' = u_B).
Proof.
  intros D u_A u_B d_A d_B k_A k_B HV HinA HinB.
  split; [apply chain_of_trust; assumption |
    split; [apply chain_of_trust; assumption |
      split; intros u' d' Hin';
        [ eapply key_owner_unique with (u := u') (u' := u_A)
        | eapply key_owner_unique with (u := u') (u' := u_B) ];
        eassumption ]].
Qed.

(* verifiable random function that maps usernames to identifiers as
   cryptographic hashes that provide privacy against dictionary attacks *)
Variable vrf : UserName -> UserIdentifier.

(* function that asserts the truth value of a mapping from name to identifier *)
Variable vrf_verify : UserName -> UserIdentifier -> Prop.

(* there are no collisions in our VRF mapping from username to useridentifier *)
Hypothesis vrf_unique :
  forall n n', vrf n = vrf n' -> n = n'.

(* anyone holding the provider's public key can check a proof of name to
   identifier *)
Hypothesis vrf_checkable :
  forall n u, vrf_verify n u -> u = vrf n.

(* a client that checked the VRF proof for n accepts only keys tracing to
   the unique root of vrf n, so the service cannot answer a lookup of n
   with a phantom entry it registered itself *)
Corollary no_phantom_identity :
  forall D n u d k,
    DirIsValid D ->
    vrf_verify n u ->
    InDir u d k D ->
    u = vrf n /\ KeyAuthentic D (vrf n) d k.
Proof.
  intros D n u d k HV Hp Hin.
  apply vrf_checkable in Hp. subst.
  split; [reflexivity | apply chain_of_trust; assumption].
Qed.

(* provider can not lie about a user's identity after their root key is uploaded
   to the directory *)
Theorem usernames_bound_to_directory :
  forall ext D1 n d_r k_r u d k,
    DirIsValid (ext ++ D1) ->
    In (D_root (vrf n) d_r k_r) D1 ->
    vrf_verify n u ->
    InDir u d k (ext ++ D1) ->
    u = vrf n /\ TracesToRoot (vrf n) d k (ext ++ D1) /\
    (forall d' k', In (D_root (vrf n) d' k') (ext ++ D1) ->
       d' = d_r /\ k' = k_r).
Proof.
  intros ext D1 n d_r k_r u d k HV Hr1 Hp Hin.
  apply vrf_checkable in Hp; subst.
  assert (Hr : In (D_root (vrf n) d_r k_r) (ext ++ D1))
    by (apply in_or_app; right; assumption).
  split; [reflexivity | split].
  - apply membership_iff_trace; assumption.
  - intros d' k' H'. eapply root_unique; eassumption.
Qed.

End KeyDirectory.
