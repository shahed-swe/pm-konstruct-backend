--
-- PostgreSQL database dump
--

\restrict S5SXzHd7jIfsxYrSAAlaiHDYyvBFj4KkmSRBZzGWIlTfeFVa3R7kMKkzhHIyHZ7

-- Dumped from database version 16.10
-- Dumped by pg_dump version 16.10

SET statement_timeout = 0;
SET lock_timeout = 0;
SET idle_in_transaction_session_timeout = 0;
SET client_encoding = 'UTF8';
SET standard_conforming_strings = on;
SELECT pg_catalog.set_config('search_path', '', false);
SET check_function_bodies = false;
SET xmloption = content;
SET client_min_messages = warning;
SET row_security = off;

--
-- Name: btree_gist; Type: EXTENSION; Schema: -; Owner: -
--

CREATE EXTENSION IF NOT EXISTS btree_gist WITH SCHEMA public;


--
-- Name: EXTENSION btree_gist; Type: COMMENT; Schema: -; Owner: -
--

COMMENT ON EXTENSION btree_gist IS 'support for indexing common datatypes in GiST';


--
-- Name: enforce_scheduler_worker_absence_no_overlap(); Type: FUNCTION; Schema: public; Owner: -
--

CREATE FUNCTION public.enforce_scheduler_worker_absence_no_overlap() RETURNS trigger
    LANGUAGE plpgsql
    AS $$
      BEGIN
        PERFORM pg_advisory_xact_lock(NEW.company_id, NEW.worker_id);
        IF EXISTS (
          SELECT 1
          FROM scheduler_worker_absences existing
          WHERE existing.company_id = NEW.company_id
            AND existing.worker_id = NEW.worker_id
            AND existing.id <> COALESCE(NEW.id, 0)
            AND daterange(existing.start_date, existing.end_date, '[]')
              && daterange(NEW.start_date, NEW.end_date, '[]')
        ) THEN
          RAISE EXCEPTION 'Worker absence periods cannot overlap'
            USING ERRCODE = '23P01';
        END IF;
        RETURN NEW;
      END;
      $$;


--
-- Name: set_updated_at(); Type: FUNCTION; Schema: public; Owner: -
--

CREATE FUNCTION public.set_updated_at() RETURNS trigger
    LANGUAGE plpgsql
    AS $$
begin
  new._updated_at = now();
  return NEW;
end;
$$;


--
-- Name: set_updated_at_metadata(); Type: FUNCTION; Schema: public; Owner: -
--

CREATE FUNCTION public.set_updated_at_metadata() RETURNS trigger
    LANGUAGE plpgsql
    AS $$
begin
  new.updated_at = now();
  return NEW;
end;
$$;


SET default_tablespace = '';

SET default_table_access_method = heap;

--
-- Name: _schema_version; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public._schema_version (
    id integer DEFAULT 1 NOT NULL,
    version integer DEFAULT 0 NOT NULL,
    CONSTRAINT _schema_single_row CHECK ((id = 1))
);


--
-- Name: ai_usage_daily; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.ai_usage_daily (
    id integer NOT NULL,
    user_id integer NOT NULL,
    date text NOT NULL,
    request_count integer DEFAULT 0 NOT NULL,
    updated_at timestamp without time zone DEFAULT now() NOT NULL
);


--
-- Name: ai_usage_daily_id_seq; Type: SEQUENCE; Schema: public; Owner: -
--

CREATE SEQUENCE public.ai_usage_daily_id_seq
    AS integer
    START WITH 1
    INCREMENT BY 1
    NO MINVALUE
    NO MAXVALUE
    CACHE 1;


--
-- Name: ai_usage_daily_id_seq; Type: SEQUENCE OWNED BY; Schema: public; Owner: -
--

ALTER SEQUENCE public.ai_usage_daily_id_seq OWNED BY public.ai_usage_daily.id;


--
-- Name: billing_webhook_notification_recipients; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.billing_webhook_notification_recipients (
    id bigint NOT NULL,
    stripe_event_id text NOT NULL,
    recipient_email text NOT NULL,
    recipient_name text NOT NULL,
    company_name text NOT NULL,
    email_kind text NOT NULL,
    trial_ends_at timestamp with time zone,
    status text DEFAULT 'pending'::text NOT NULL,
    attempt_count integer DEFAULT 0 NOT NULL,
    next_attempt_at timestamp with time zone,
    last_attempt_at timestamp with time zone,
    delivered_at timestamp with time zone,
    last_error text,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    operator_alerted_at timestamp with time zone,
    claim_token text,
    claim_expires_at timestamp with time zone,
    ambiguous_since timestamp with time zone,
    CONSTRAINT billing_webhook_notification_recipients_status_check CHECK ((status = ANY (ARRAY['pending'::text, 'processing'::text, 'delivered'::text, 'failed'::text, 'review'::text])))
);


--
-- Name: billing_webhook_notification_recipients_id_seq; Type: SEQUENCE; Schema: public; Owner: -
--

CREATE SEQUENCE public.billing_webhook_notification_recipients_id_seq
    START WITH 1
    INCREMENT BY 1
    NO MINVALUE
    NO MAXVALUE
    CACHE 1;


--
-- Name: billing_webhook_notification_recipients_id_seq; Type: SEQUENCE OWNED BY; Schema: public; Owner: -
--

ALTER SEQUENCE public.billing_webhook_notification_recipients_id_seq OWNED BY public.billing_webhook_notification_recipients.id;


--
-- Name: billing_webhook_notifications; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.billing_webhook_notifications (
    stripe_event_id text NOT NULL,
    event_type text NOT NULL,
    status text DEFAULT 'pending'::text NOT NULL,
    last_error text,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    delivered_at timestamp with time zone,
    CONSTRAINT billing_webhook_notifications_status_check CHECK ((status = ANY (ARRAY['pending'::text, 'delivered'::text, 'failed'::text])))
);


--
-- Name: call_forward; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.call_forward (
    id integer NOT NULL,
    job_id integer NOT NULL,
    title text NOT NULL,
    item_type character varying(20) DEFAULT 'TASK'::character varying NOT NULL,
    supplier_trade character varying(150),
    est_start date,
    est_finish date,
    actual_start date,
    actual_finish date,
    status character varying(30) DEFAULT 'not_started'::character varying NOT NULL,
    notes text,
    sort_order integer DEFAULT 0 NOT NULL,
    parent_id integer,
    created_at timestamp without time zone DEFAULT now() NOT NULL,
    updated_at timestamp without time zone DEFAULT now() NOT NULL
);


--
-- Name: call_forward_id_seq; Type: SEQUENCE; Schema: public; Owner: -
--

CREATE SEQUENCE public.call_forward_id_seq
    AS integer
    START WITH 1
    INCREMENT BY 1
    NO MINVALUE
    NO MAXVALUE
    CACHE 1;


--
-- Name: call_forward_id_seq; Type: SEQUENCE OWNED BY; Schema: public; Owner: -
--

ALTER SEQUENCE public.call_forward_id_seq OWNED BY public.call_forward.id;


--
-- Name: call_forward_templates; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.call_forward_templates (
    id integer NOT NULL,
    company_id integer NOT NULL,
    name text NOT NULL,
    description text,
    items jsonb DEFAULT '[]'::jsonb NOT NULL,
    created_at timestamp without time zone DEFAULT now() NOT NULL,
    updated_at timestamp without time zone DEFAULT now() NOT NULL
);


--
-- Name: call_forward_templates_id_seq; Type: SEQUENCE; Schema: public; Owner: -
--

CREATE SEQUENCE public.call_forward_templates_id_seq
    AS integer
    START WITH 1
    INCREMENT BY 1
    NO MINVALUE
    NO MAXVALUE
    CACHE 1;


--
-- Name: call_forward_templates_id_seq; Type: SEQUENCE OWNED BY; Schema: public; Owner: -
--

ALTER SEQUENCE public.call_forward_templates_id_seq OWNED BY public.call_forward_templates.id;


--
-- Name: companies; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.companies (
    id integer NOT NULL,
    name text NOT NULL,
    created_at timestamp without time zone DEFAULT now() NOT NULL,
    stripe_customer_id text,
    stripe_subscription_id text,
    billing_price_id text,
    billing_plan_key text,
    billing_seat_quantity integer,
    billing_trial_ends_at timestamp without time zone,
    billing_onboarding_completed boolean DEFAULT true NOT NULL
);


--
-- Name: companies_id_seq; Type: SEQUENCE; Schema: public; Owner: -
--

CREATE SEQUENCE public.companies_id_seq
    AS integer
    START WITH 1
    INCREMENT BY 1
    NO MINVALUE
    NO MAXVALUE
    CACHE 1;


--
-- Name: companies_id_seq; Type: SEQUENCE OWNED BY; Schema: public; Owner: -
--

ALTER SEQUENCE public.companies_id_seq OWNED BY public.companies.id;


--
-- Name: company_branding; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.company_branding (
    id integer NOT NULL,
    company_name text DEFAULT 'PM Konstruct'::text NOT NULL,
    logo_url text,
    primary_color text DEFAULT '#E84E1B'::text NOT NULL,
    sidebar_color text DEFAULT '#0f1117'::text NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL,
    company_id integer NOT NULL,
    banner_url text,
    job_display_mode text DEFAULT 'job_number'::text NOT NULL,
    email_send_mode text DEFAULT 'device'::text NOT NULL
);


--
-- Name: company_branding_id_seq; Type: SEQUENCE; Schema: public; Owner: -
--

CREATE SEQUENCE public.company_branding_id_seq
    AS integer
    START WITH 1
    INCREMENT BY 1
    NO MINVALUE
    NO MAXVALUE
    CACHE 1;


--
-- Name: company_branding_id_seq; Type: SEQUENCE OWNED BY; Schema: public; Owner: -
--

ALTER SEQUENCE public.company_branding_id_seq OWNED BY public.company_branding.id;


--
-- Name: diary_media; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.diary_media (
    id integer NOT NULL,
    diary_entry_id integer NOT NULL,
    file_type character varying(10) NOT NULL,
    mime_type character varying(100) NOT NULL,
    original_name character varying(255) NOT NULL,
    stored_name character varying(255) NOT NULL,
    file_size integer NOT NULL,
    url character varying(500) NOT NULL,
    uploaded_by integer,
    created_at timestamp without time zone DEFAULT now() NOT NULL,
    note_id integer
);


--
-- Name: diary_media_id_seq; Type: SEQUENCE; Schema: public; Owner: -
--

CREATE SEQUENCE public.diary_media_id_seq
    AS integer
    START WITH 1
    INCREMENT BY 1
    NO MINVALUE
    NO MAXVALUE
    CACHE 1;


--
-- Name: diary_media_id_seq; Type: SEQUENCE OWNED BY; Schema: public; Owner: -
--

ALTER SEQUENCE public.diary_media_id_seq OWNED BY public.diary_media.id;


--
-- Name: diary_note_comments; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.diary_note_comments (
    id integer NOT NULL,
    note_id integer NOT NULL,
    author_id integer,
    content text NOT NULL,
    created_at timestamp without time zone DEFAULT now() NOT NULL
);


--
-- Name: diary_note_comments_id_seq; Type: SEQUENCE; Schema: public; Owner: -
--

CREATE SEQUENCE public.diary_note_comments_id_seq
    AS integer
    START WITH 1
    INCREMENT BY 1
    NO MINVALUE
    NO MAXVALUE
    CACHE 1;


--
-- Name: diary_note_comments_id_seq; Type: SEQUENCE OWNED BY; Schema: public; Owner: -
--

ALTER SEQUENCE public.diary_note_comments_id_seq OWNED BY public.diary_note_comments.id;


--
-- Name: diary_notes; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.diary_notes (
    id integer NOT NULL,
    diary_entry_id integer NOT NULL,
    category text DEFAULT 'general'::text NOT NULL,
    content text NOT NULL,
    action_status text,
    action_raised_by integer,
    sort_order integer DEFAULT 0 NOT NULL,
    created_at timestamp without time zone DEFAULT now() NOT NULL,
    archived boolean DEFAULT false NOT NULL
);


--
-- Name: diary_notes_id_seq; Type: SEQUENCE; Schema: public; Owner: -
--

CREATE SEQUENCE public.diary_notes_id_seq
    AS integer
    START WITH 1
    INCREMENT BY 1
    NO MINVALUE
    NO MAXVALUE
    CACHE 1;


--
-- Name: diary_notes_id_seq; Type: SEQUENCE OWNED BY; Schema: public; Owner: -
--

ALTER SEQUENCE public.diary_notes_id_seq OWNED BY public.diary_notes.id;


--
-- Name: email_settings; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.email_settings (
    id integer NOT NULL,
    smtp_host text,
    smtp_port integer DEFAULT 587,
    smtp_user text,
    smtp_pass text,
    smtp_from text,
    smtp_secure boolean DEFAULT false,
    updated_at timestamp with time zone DEFAULT now() NOT NULL,
    company_id integer NOT NULL
);


--
-- Name: email_settings_id_seq; Type: SEQUENCE; Schema: public; Owner: -
--

CREATE SEQUENCE public.email_settings_id_seq
    AS integer
    START WITH 1
    INCREMENT BY 1
    NO MINVALUE
    NO MAXVALUE
    CACHE 1;


--
-- Name: email_settings_id_seq; Type: SEQUENCE OWNED BY; Schema: public; Owner: -
--

ALTER SEQUENCE public.email_settings_id_seq OWNED BY public.email_settings.id;


--
-- Name: eto_job_sequences; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.eto_job_sequences (
    job_id integer NOT NULL,
    next_number integer DEFAULT 1 NOT NULL,
    CONSTRAINT eto_job_sequences_next_number_check CHECK ((next_number > 0))
);


--
-- Name: inspection_form_items; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.inspection_form_items (
    id integer NOT NULL,
    form_id integer NOT NULL,
    client_key character varying(100) NOT NULL,
    room text DEFAULT ''::text NOT NULL,
    description text DEFAULT ''::text NOT NULL,
    actioned boolean DEFAULT false NOT NULL,
    sort_order integer DEFAULT 0 NOT NULL
);


--
-- Name: inspection_form_items_id_seq; Type: SEQUENCE; Schema: public; Owner: -
--

CREATE SEQUENCE public.inspection_form_items_id_seq
    AS integer
    START WITH 1
    INCREMENT BY 1
    NO MINVALUE
    NO MAXVALUE
    CACHE 1;


--
-- Name: inspection_form_items_id_seq; Type: SEQUENCE OWNED BY; Schema: public; Owner: -
--

ALTER SEQUENCE public.inspection_form_items_id_seq OWNED BY public.inspection_form_items.id;


--
-- Name: inspection_form_media; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.inspection_form_media (
    id integer NOT NULL,
    form_id integer NOT NULL,
    item_id integer NOT NULL,
    diary_media_id integer NOT NULL,
    sort_order integer DEFAULT 0 NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL
);


--
-- Name: inspection_form_media_id_seq; Type: SEQUENCE; Schema: public; Owner: -
--

CREATE SEQUENCE public.inspection_form_media_id_seq
    AS integer
    START WITH 1
    INCREMENT BY 1
    NO MINVALUE
    NO MAXVALUE
    CACHE 1;


--
-- Name: inspection_form_media_id_seq; Type: SEQUENCE OWNED BY; Schema: public; Owner: -
--

ALTER SEQUENCE public.inspection_form_media_id_seq OWNED BY public.inspection_form_media.id;


--
-- Name: inspection_forms; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.inspection_forms (
    id integer NOT NULL,
    company_id integer NOT NULL,
    job_id integer NOT NULL,
    created_by integer NOT NULL,
    diary_entry_id integer NOT NULL,
    diary_note_id integer NOT NULL,
    inspection_date date NOT NULL,
    inspector text DEFAULT ''::text NOT NULL,
    inspection_type text DEFAULT ''::text NOT NULL,
    stage text DEFAULT ''::text NOT NULL,
    observations text DEFAULT ''::text NOT NULL,
    weather_data jsonb,
    revision integer DEFAULT 0 NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL
);


--
-- Name: inspection_forms_id_seq; Type: SEQUENCE; Schema: public; Owner: -
--

CREATE SEQUENCE public.inspection_forms_id_seq
    AS integer
    START WITH 1
    INCREMENT BY 1
    NO MINVALUE
    NO MAXVALUE
    CACHE 1;


--
-- Name: inspection_forms_id_seq; Type: SEQUENCE OWNED BY; Schema: public; Owner: -
--

ALTER SEQUENCE public.inspection_forms_id_seq OWNED BY public.inspection_forms.id;


--
-- Name: job_assignments; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.job_assignments (
    id integer NOT NULL,
    job_id integer NOT NULL,
    user_id integer NOT NULL,
    is_primary boolean DEFAULT false NOT NULL,
    assigned_at timestamp without time zone DEFAULT now() NOT NULL
);


--
-- Name: job_assignments_id_seq; Type: SEQUENCE; Schema: public; Owner: -
--

CREATE SEQUENCE public.job_assignments_id_seq
    AS integer
    START WITH 1
    INCREMENT BY 1
    NO MINVALUE
    NO MAXVALUE
    CACHE 1;


--
-- Name: job_assignments_id_seq; Type: SEQUENCE OWNED BY; Schema: public; Owner: -
--

ALTER SEQUENCE public.job_assignments_id_seq OWNED BY public.job_assignments.id;


--
-- Name: job_dropbox_auto_uploads; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.job_dropbox_auto_uploads (
    id integer NOT NULL,
    job_id integer NOT NULL,
    company_id integer NOT NULL,
    folder_id integer NOT NULL,
    enabled boolean DEFAULT true NOT NULL,
    last_run_date date,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL,
    frequency character varying(10) DEFAULT 'daily'::character varying NOT NULL,
    last_run_at timestamp with time zone,
    run_claimed_at timestamp with time zone
);


--
-- Name: job_dropbox_auto_uploads_id_seq; Type: SEQUENCE; Schema: public; Owner: -
--

CREATE SEQUENCE public.job_dropbox_auto_uploads_id_seq
    AS integer
    START WITH 1
    INCREMENT BY 1
    NO MINVALUE
    NO MAXVALUE
    CACHE 1;


--
-- Name: job_dropbox_auto_uploads_id_seq; Type: SEQUENCE OWNED BY; Schema: public; Owner: -
--

ALTER SEQUENCE public.job_dropbox_auto_uploads_id_seq OWNED BY public.job_dropbox_auto_uploads.id;


--
-- Name: job_dropbox_folders; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.job_dropbox_folders (
    id integer NOT NULL,
    job_id integer NOT NULL,
    company_id integer NOT NULL,
    label text DEFAULT 'Dropbox Files'::text NOT NULL,
    path text NOT NULL,
    sort_order integer DEFAULT 0 NOT NULL,
    created_at timestamp without time zone DEFAULT now() NOT NULL
);


--
-- Name: job_dropbox_folders_id_seq; Type: SEQUENCE; Schema: public; Owner: -
--

CREATE SEQUENCE public.job_dropbox_folders_id_seq
    AS integer
    START WITH 1
    INCREMENT BY 1
    NO MINVALUE
    NO MAXVALUE
    CACHE 1;


--
-- Name: job_dropbox_folders_id_seq; Type: SEQUENCE OWNED BY; Schema: public; Owner: -
--

ALTER SEQUENCE public.job_dropbox_folders_id_seq OWNED BY public.job_dropbox_folders.id;


--
-- Name: job_dropbox_uploads; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.job_dropbox_uploads (
    id integer NOT NULL,
    job_id integer NOT NULL,
    company_id integer NOT NULL,
    folder_id integer NOT NULL,
    media_source character varying(12) NOT NULL,
    media_id integer NOT NULL,
    dropbox_path text,
    status character varying(12) DEFAULT 'pending'::character varying NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    completed_at timestamp with time zone
);


--
-- Name: job_dropbox_uploads_id_seq; Type: SEQUENCE; Schema: public; Owner: -
--

CREATE SEQUENCE public.job_dropbox_uploads_id_seq
    AS integer
    START WITH 1
    INCREMENT BY 1
    NO MINVALUE
    NO MAXVALUE
    CACHE 1;


--
-- Name: job_dropbox_uploads_id_seq; Type: SEQUENCE OWNED BY; Schema: public; Owner: -
--

ALTER SEQUENCE public.job_dropbox_uploads_id_seq OWNED BY public.job_dropbox_uploads.id;


--
-- Name: job_media; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.job_media (
    id integer NOT NULL,
    job_id integer NOT NULL,
    file_type character varying(10) DEFAULT 'photo'::character varying NOT NULL,
    mime_type character varying(100) NOT NULL,
    original_name character varying(255) NOT NULL,
    stored_name character varying(255) NOT NULL,
    file_size integer NOT NULL,
    url character varying(500) NOT NULL,
    source character varying(30) DEFAULT 'upload'::character varying NOT NULL,
    source_path text,
    uploaded_by integer,
    created_at timestamp with time zone DEFAULT now() NOT NULL
);


--
-- Name: job_media_id_seq; Type: SEQUENCE; Schema: public; Owner: -
--

CREATE SEQUENCE public.job_media_id_seq
    AS integer
    START WITH 1
    INCREMENT BY 1
    NO MINVALUE
    NO MAXVALUE
    CACHE 1;


--
-- Name: job_media_id_seq; Type: SEQUENCE OWNED BY; Schema: public; Owner: -
--

ALTER SEQUENCE public.job_media_id_seq OWNED BY public.job_media.id;


--
-- Name: job_tasks; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.job_tasks (
    id integer NOT NULL,
    job_id integer NOT NULL,
    title text NOT NULL,
    status text DEFAULT 'pending'::text NOT NULL,
    notes text,
    sort_order integer DEFAULT 0 NOT NULL,
    created_at timestamp without time zone DEFAULT now() NOT NULL,
    updated_at timestamp without time zone DEFAULT now() NOT NULL
);


--
-- Name: job_tasks_id_seq; Type: SEQUENCE; Schema: public; Owner: -
--

CREATE SEQUENCE public.job_tasks_id_seq
    AS integer
    START WITH 1
    INCREMENT BY 1
    NO MINVALUE
    NO MAXVALUE
    CACHE 1;


--
-- Name: job_tasks_id_seq; Type: SEQUENCE OWNED BY; Schema: public; Owner: -
--

ALTER SEQUENCE public.job_tasks_id_seq OWNED BY public.job_tasks.id;


--
-- Name: jobs; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.jobs (
    id integer NOT NULL,
    name text NOT NULL,
    job_number text NOT NULL,
    client text NOT NULL,
    address text NOT NULL,
    status text DEFAULT 'active'::text NOT NULL,
    start_date date,
    end_date date,
    description text,
    created_at timestamp without time zone DEFAULT now() NOT NULL,
    updated_at timestamp without time zone DEFAULT now() NOT NULL,
    client_number text,
    manager_id integer,
    supervisor_id integer,
    dropbox_path text,
    company_id integer NOT NULL,
    client_email text,
    contact2_name text,
    contact2_phone text,
    contact2_email text
);


--
-- Name: jobs_id_seq; Type: SEQUENCE; Schema: public; Owner: -
--

CREATE SEQUENCE public.jobs_id_seq
    AS integer
    START WITH 1
    INCREMENT BY 1
    NO MINVALUE
    NO MAXVALUE
    CACHE 1;


--
-- Name: jobs_id_seq; Type: SEQUENCE OWNED BY; Schema: public; Owner: -
--

ALTER SEQUENCE public.jobs_id_seq OWNED BY public.jobs.id;


--
-- Name: media_deletion_queue; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.media_deletion_queue (
    id integer NOT NULL,
    stored_name character varying(255) NOT NULL,
    attempts integer DEFAULT 0 NOT NULL,
    last_error text,
    last_attempt_at timestamp with time zone,
    created_at timestamp with time zone DEFAULT now() NOT NULL
);


--
-- Name: media_deletion_queue_id_seq; Type: SEQUENCE; Schema: public; Owner: -
--

CREATE SEQUENCE public.media_deletion_queue_id_seq
    AS integer
    START WITH 1
    INCREMENT BY 1
    NO MINVALUE
    NO MAXVALUE
    CACHE 1;


--
-- Name: media_deletion_queue_id_seq; Type: SEQUENCE OWNED BY; Schema: public; Owner: -
--

ALTER SEQUENCE public.media_deletion_queue_id_seq OWNED BY public.media_deletion_queue.id;


--
-- Name: notifications; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.notifications (
    id integer NOT NULL,
    user_id integer NOT NULL,
    type character varying(60) NOT NULL,
    title text NOT NULL,
    body text,
    link text,
    read_at timestamp without time zone,
    created_at timestamp without time zone DEFAULT now() NOT NULL,
    resolved_at timestamp with time zone
);


--
-- Name: notifications_id_seq; Type: SEQUENCE; Schema: public; Owner: -
--

CREATE SEQUENCE public.notifications_id_seq
    AS integer
    START WITH 1
    INCREMENT BY 1
    NO MINVALUE
    NO MAXVALUE
    CACHE 1;


--
-- Name: notifications_id_seq; Type: SEQUENCE OWNED BY; Schema: public; Owner: -
--

ALTER SEQUENCE public.notifications_id_seq OWNED BY public.notifications.id;


--
-- Name: password_reset_tokens; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.password_reset_tokens (
    id integer NOT NULL,
    user_id integer NOT NULL,
    token text NOT NULL,
    expires_at timestamp with time zone NOT NULL,
    used_at timestamp with time zone,
    created_at timestamp with time zone DEFAULT now() NOT NULL
);


--
-- Name: password_reset_tokens_id_seq; Type: SEQUENCE; Schema: public; Owner: -
--

CREATE SEQUENCE public.password_reset_tokens_id_seq
    AS integer
    START WITH 1
    INCREMENT BY 1
    NO MINVALUE
    NO MAXVALUE
    CACHE 1;


--
-- Name: password_reset_tokens_id_seq; Type: SEQUENCE OWNED BY; Schema: public; Owner: -
--

ALTER SEQUENCE public.password_reset_tokens_id_seq OWNED BY public.password_reset_tokens.id;


--
-- Name: progress; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.progress (
    id integer NOT NULL,
    job_id integer NOT NULL,
    date date NOT NULL,
    percent_complete real NOT NULL,
    milestone text,
    description text,
    photos text[],
    created_at timestamp without time zone DEFAULT now() NOT NULL,
    updated_at timestamp without time zone DEFAULT now() NOT NULL
);


--
-- Name: progress_id_seq; Type: SEQUENCE; Schema: public; Owner: -
--

CREATE SEQUENCE public.progress_id_seq
    AS integer
    START WITH 1
    INCREMENT BY 1
    NO MINVALUE
    NO MAXVALUE
    CACHE 1;


--
-- Name: progress_id_seq; Type: SEQUENCE OWNED BY; Schema: public; Owner: -
--

ALTER SEQUENCE public.progress_id_seq OWNED BY public.progress.id;


--
-- Name: push_subscriptions; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.push_subscriptions (
    id integer NOT NULL,
    user_id integer NOT NULL,
    endpoint text NOT NULL,
    p256dh text NOT NULL,
    auth text NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL
);


--
-- Name: push_subscriptions_id_seq; Type: SEQUENCE; Schema: public; Owner: -
--

CREATE SEQUENCE public.push_subscriptions_id_seq
    AS integer
    START WITH 1
    INCREMENT BY 1
    NO MINVALUE
    NO MAXVALUE
    CACHE 1;


--
-- Name: push_subscriptions_id_seq; Type: SEQUENCE OWNED BY; Schema: public; Owner: -
--

ALTER SEQUENCE public.push_subscriptions_id_seq OWNED BY public.push_subscriptions.id;


--
-- Name: reports; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.reports (
    id integer NOT NULL,
    job_id integer NOT NULL,
    title text NOT NULL,
    type text NOT NULL,
    content text NOT NULL,
    generated_at timestamp without time zone DEFAULT now() NOT NULL,
    created_at timestamp without time zone DEFAULT now() NOT NULL
);


--
-- Name: reports_id_seq; Type: SEQUENCE; Schema: public; Owner: -
--

CREATE SEQUENCE public.reports_id_seq
    AS integer
    START WITH 1
    INCREMENT BY 1
    NO MINVALUE
    NO MAXVALUE
    CACHE 1;


--
-- Name: reports_id_seq; Type: SEQUENCE OWNED BY; Schema: public; Owner: -
--

ALTER SEQUENCE public.reports_id_seq OWNED BY public.reports.id;


--
-- Name: scheduler_allocations; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.scheduler_allocations (
    id integer NOT NULL,
    company_id integer NOT NULL,
    worker_id integer NOT NULL,
    job_id integer,
    assigned_date date NOT NULL,
    note text,
    created_at timestamp without time zone DEFAULT now() NOT NULL,
    updated_at timestamp without time zone DEFAULT now() NOT NULL,
    maintenance_job_id integer,
    CONSTRAINT scheduler_allocation_target_check CHECK ((((job_id IS NOT NULL) AND (maintenance_job_id IS NULL)) OR ((job_id IS NULL) AND (maintenance_job_id IS NOT NULL))))
);


--
-- Name: scheduler_allocations_id_seq; Type: SEQUENCE; Schema: public; Owner: -
--

CREATE SEQUENCE public.scheduler_allocations_id_seq
    AS integer
    START WITH 1
    INCREMENT BY 1
    NO MINVALUE
    NO MAXVALUE
    CACHE 1;


--
-- Name: scheduler_allocations_id_seq; Type: SEQUENCE OWNED BY; Schema: public; Owner: -
--

ALTER SEQUENCE public.scheduler_allocations_id_seq OWNED BY public.scheduler_allocations.id;


--
-- Name: scheduler_job_day_notes; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.scheduler_job_day_notes (
    id integer NOT NULL,
    company_id integer NOT NULL,
    job_id integer NOT NULL,
    note_date date NOT NULL,
    note text NOT NULL,
    created_at timestamp without time zone DEFAULT now() NOT NULL,
    updated_at timestamp without time zone DEFAULT now() NOT NULL
);


--
-- Name: scheduler_job_day_notes_id_seq; Type: SEQUENCE; Schema: public; Owner: -
--

CREATE SEQUENCE public.scheduler_job_day_notes_id_seq
    AS integer
    START WITH 1
    INCREMENT BY 1
    NO MINVALUE
    NO MAXVALUE
    CACHE 1;


--
-- Name: scheduler_job_day_notes_id_seq; Type: SEQUENCE OWNED BY; Schema: public; Owner: -
--

ALTER SEQUENCE public.scheduler_job_day_notes_id_seq OWNED BY public.scheduler_job_day_notes.id;


--
-- Name: scheduler_maintenance_jobs; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.scheduler_maintenance_jobs (
    id integer NOT NULL,
    company_id integer NOT NULL,
    name text NOT NULL,
    reference text,
    address text,
    status text DEFAULT 'active'::text NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL
);


--
-- Name: scheduler_maintenance_jobs_id_seq; Type: SEQUENCE; Schema: public; Owner: -
--

CREATE SEQUENCE public.scheduler_maintenance_jobs_id_seq
    AS integer
    START WITH 1
    INCREMENT BY 1
    NO MINVALUE
    NO MAXVALUE
    CACHE 1;


--
-- Name: scheduler_maintenance_jobs_id_seq; Type: SEQUENCE OWNED BY; Schema: public; Owner: -
--

ALTER SEQUENCE public.scheduler_maintenance_jobs_id_seq OWNED BY public.scheduler_maintenance_jobs.id;


--
-- Name: scheduler_worker_absences; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.scheduler_worker_absences (
    id integer NOT NULL,
    company_id integer NOT NULL,
    worker_id integer NOT NULL,
    absence_type text NOT NULL,
    start_date date NOT NULL,
    end_date date NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL,
    CONSTRAINT scheduler_worker_absence_dates_check CHECK ((start_date <= end_date)),
    CONSTRAINT scheduler_worker_absence_type_check CHECK ((absence_type = ANY (ARRAY['sick'::text, 'leave'::text, 'trade_school'::text])))
);


--
-- Name: scheduler_worker_absences_id_seq; Type: SEQUENCE; Schema: public; Owner: -
--

CREATE SEQUENCE public.scheduler_worker_absences_id_seq
    AS integer
    START WITH 1
    INCREMENT BY 1
    NO MINVALUE
    NO MAXVALUE
    CACHE 1;


--
-- Name: scheduler_worker_absences_id_seq; Type: SEQUENCE OWNED BY; Schema: public; Owner: -
--

ALTER SEQUENCE public.scheduler_worker_absences_id_seq OWNED BY public.scheduler_worker_absences.id;


--
-- Name: scheduler_workers; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.scheduler_workers (
    id integer NOT NULL,
    company_id integer NOT NULL,
    name text NOT NULL,
    trade text,
    color text DEFAULT '#3B82F6'::text NOT NULL,
    active boolean DEFAULT true NOT NULL,
    created_at timestamp without time zone DEFAULT now() NOT NULL,
    updated_at timestamp without time zone DEFAULT now() NOT NULL,
    on_leave boolean DEFAULT false NOT NULL,
    leave_from date,
    leave_to date
);


--
-- Name: scheduler_workers_id_seq; Type: SEQUENCE; Schema: public; Owner: -
--

CREATE SEQUENCE public.scheduler_workers_id_seq
    AS integer
    START WITH 1
    INCREMENT BY 1
    NO MINVALUE
    NO MAXVALUE
    CACHE 1;


--
-- Name: scheduler_workers_id_seq; Type: SEQUENCE OWNED BY; Schema: public; Owner: -
--

ALTER SEQUENCE public.scheduler_workers_id_seq OWNED BY public.scheduler_workers.id;


--
-- Name: site_diary; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.site_diary (
    id integer NOT NULL,
    job_id integer NOT NULL,
    date date NOT NULL,
    weather text,
    workforce integer,
    work_completed text NOT NULL,
    materials text,
    equipment text,
    visitors text,
    issues text,
    notes text,
    created_at timestamp without time zone DEFAULT now() NOT NULL,
    updated_at timestamp without time zone DEFAULT now() NOT NULL,
    "time" text,
    author_id integer,
    trades_on_site text,
    safety_notes text,
    client_instructions text,
    location_name text,
    location_lat numeric(9,6),
    location_lng numeric(9,6),
    temperature numeric(5,1),
    weather_condition text,
    weather_icon text,
    wind_speed_kmh numeric(6,1),
    rainfall_mm numeric(6,1),
    sunrise_time text,
    sunset_time text,
    action_status text,
    action_raised_by integer
);


--
-- Name: site_diary_id_seq; Type: SEQUENCE; Schema: public; Owner: -
--

CREATE SEQUENCE public.site_diary_id_seq
    AS integer
    START WITH 1
    INCREMENT BY 1
    NO MINVALUE
    NO MAXVALUE
    CACHE 1;


--
-- Name: site_diary_id_seq; Type: SEQUENCE OWNED BY; Schema: public; Owner: -
--

ALTER SEQUENCE public.site_diary_id_seq OWNED BY public.site_diary.id;


--
-- Name: supabase_import_mappings; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.supabase_import_mappings (
    source_origin text NOT NULL,
    entity text NOT NULL,
    source_id integer NOT NULL,
    target_id integer NOT NULL,
    created_at timestamp with time zone DEFAULT now() NOT NULL
);


--
-- Name: user_notification_prefs; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.user_notification_prefs (
    id integer NOT NULL,
    user_id integer NOT NULL,
    notify_action_notes boolean DEFAULT true NOT NULL,
    notify_call_forward boolean DEFAULT false NOT NULL,
    updated_at timestamp with time zone DEFAULT now() NOT NULL
);


--
-- Name: user_notification_prefs_id_seq; Type: SEQUENCE; Schema: public; Owner: -
--

CREATE SEQUENCE public.user_notification_prefs_id_seq
    AS integer
    START WITH 1
    INCREMENT BY 1
    NO MINVALUE
    NO MAXVALUE
    CACHE 1;


--
-- Name: user_notification_prefs_id_seq; Type: SEQUENCE OWNED BY; Schema: public; Owner: -
--

ALTER SEQUENCE public.user_notification_prefs_id_seq OWNED BY public.user_notification_prefs.id;


--
-- Name: user_permissions; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.user_permissions (
    id integer NOT NULL,
    user_id integer NOT NULL,
    resource character varying(100) NOT NULL,
    action character varying(100) NOT NULL
);


--
-- Name: user_permissions_id_seq; Type: SEQUENCE; Schema: public; Owner: -
--

CREATE SEQUENCE public.user_permissions_id_seq
    AS integer
    START WITH 1
    INCREMENT BY 1
    NO MINVALUE
    NO MAXVALUE
    CACHE 1;


--
-- Name: user_permissions_id_seq; Type: SEQUENCE OWNED BY; Schema: public; Owner: -
--

ALTER SEQUENCE public.user_permissions_id_seq OWNED BY public.user_permissions.id;


--
-- Name: users; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.users (
    id integer NOT NULL,
    name text NOT NULL,
    email text NOT NULL,
    role text DEFAULT 'SUPERVISOR'::text NOT NULL,
    phone text,
    active boolean DEFAULT true NOT NULL,
    created_at timestamp without time zone DEFAULT now() NOT NULL,
    updated_at timestamp without time zone DEFAULT now() NOT NULL,
    password_hash text,
    company_id integer NOT NULL,
    password_changed_at timestamp without time zone
);


--
-- Name: users_id_seq; Type: SEQUENCE; Schema: public; Owner: -
--

CREATE SEQUENCE public.users_id_seq
    AS integer
    START WITH 1
    INCREMENT BY 1
    NO MINVALUE
    NO MAXVALUE
    CACHE 1;


--
-- Name: users_id_seq; Type: SEQUENCE OWNED BY; Schema: public; Owner: -
--

ALTER SEQUENCE public.users_id_seq OWNED BY public.users.id;


--
-- Name: weather_snapshots; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.weather_snapshots (
    id integer NOT NULL,
    diary_entry_id integer NOT NULL,
    temperature_c numeric(5,1),
    conditions character varying(100),
    rain character varying(20) DEFAULT 'none'::character varying,
    wind_description character varying(100),
    wind_speed_kmh numeric(5,1),
    humidity_pct integer,
    source character varying(20) DEFAULT 'manual'::character varying NOT NULL,
    snapshot_at timestamp without time zone DEFAULT now() NOT NULL,
    created_at timestamp without time zone DEFAULT now() NOT NULL
);


--
-- Name: weather_snapshots_id_seq; Type: SEQUENCE; Schema: public; Owner: -
--

CREATE SEQUENCE public.weather_snapshots_id_seq
    AS integer
    START WITH 1
    INCREMENT BY 1
    NO MINVALUE
    NO MAXVALUE
    CACHE 1;


--
-- Name: weather_snapshots_id_seq; Type: SEQUENCE OWNED BY; Schema: public; Owner: -
--

ALTER SEQUENCE public.weather_snapshots_id_seq OWNED BY public.weather_snapshots.id;


--
-- Name: ai_usage_daily id; Type: DEFAULT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.ai_usage_daily ALTER COLUMN id SET DEFAULT nextval('public.ai_usage_daily_id_seq'::regclass);


--
-- Name: billing_webhook_notification_recipients id; Type: DEFAULT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.billing_webhook_notification_recipients ALTER COLUMN id SET DEFAULT nextval('public.billing_webhook_notification_recipients_id_seq'::regclass);


--
-- Name: call_forward id; Type: DEFAULT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.call_forward ALTER COLUMN id SET DEFAULT nextval('public.call_forward_id_seq'::regclass);


--
-- Name: call_forward_templates id; Type: DEFAULT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.call_forward_templates ALTER COLUMN id SET DEFAULT nextval('public.call_forward_templates_id_seq'::regclass);


--
-- Name: companies id; Type: DEFAULT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.companies ALTER COLUMN id SET DEFAULT nextval('public.companies_id_seq'::regclass);


--
-- Name: company_branding id; Type: DEFAULT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.company_branding ALTER COLUMN id SET DEFAULT nextval('public.company_branding_id_seq'::regclass);


--
-- Name: diary_media id; Type: DEFAULT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.diary_media ALTER COLUMN id SET DEFAULT nextval('public.diary_media_id_seq'::regclass);


--
-- Name: diary_note_comments id; Type: DEFAULT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.diary_note_comments ALTER COLUMN id SET DEFAULT nextval('public.diary_note_comments_id_seq'::regclass);


--
-- Name: diary_notes id; Type: DEFAULT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.diary_notes ALTER COLUMN id SET DEFAULT nextval('public.diary_notes_id_seq'::regclass);


--
-- Name: email_settings id; Type: DEFAULT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.email_settings ALTER COLUMN id SET DEFAULT nextval('public.email_settings_id_seq'::regclass);


--
-- Name: inspection_form_items id; Type: DEFAULT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.inspection_form_items ALTER COLUMN id SET DEFAULT nextval('public.inspection_form_items_id_seq'::regclass);


--
-- Name: inspection_form_media id; Type: DEFAULT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.inspection_form_media ALTER COLUMN id SET DEFAULT nextval('public.inspection_form_media_id_seq'::regclass);


--
-- Name: inspection_forms id; Type: DEFAULT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.inspection_forms ALTER COLUMN id SET DEFAULT nextval('public.inspection_forms_id_seq'::regclass);


--
-- Name: job_assignments id; Type: DEFAULT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.job_assignments ALTER COLUMN id SET DEFAULT nextval('public.job_assignments_id_seq'::regclass);


--
-- Name: job_dropbox_auto_uploads id; Type: DEFAULT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.job_dropbox_auto_uploads ALTER COLUMN id SET DEFAULT nextval('public.job_dropbox_auto_uploads_id_seq'::regclass);


--
-- Name: job_dropbox_folders id; Type: DEFAULT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.job_dropbox_folders ALTER COLUMN id SET DEFAULT nextval('public.job_dropbox_folders_id_seq'::regclass);


--
-- Name: job_dropbox_uploads id; Type: DEFAULT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.job_dropbox_uploads ALTER COLUMN id SET DEFAULT nextval('public.job_dropbox_uploads_id_seq'::regclass);


--
-- Name: job_media id; Type: DEFAULT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.job_media ALTER COLUMN id SET DEFAULT nextval('public.job_media_id_seq'::regclass);


--
-- Name: job_tasks id; Type: DEFAULT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.job_tasks ALTER COLUMN id SET DEFAULT nextval('public.job_tasks_id_seq'::regclass);


--
-- Name: jobs id; Type: DEFAULT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.jobs ALTER COLUMN id SET DEFAULT nextval('public.jobs_id_seq'::regclass);


--
-- Name: media_deletion_queue id; Type: DEFAULT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.media_deletion_queue ALTER COLUMN id SET DEFAULT nextval('public.media_deletion_queue_id_seq'::regclass);


--
-- Name: notifications id; Type: DEFAULT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.notifications ALTER COLUMN id SET DEFAULT nextval('public.notifications_id_seq'::regclass);


--
-- Name: password_reset_tokens id; Type: DEFAULT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.password_reset_tokens ALTER COLUMN id SET DEFAULT nextval('public.password_reset_tokens_id_seq'::regclass);


--
-- Name: progress id; Type: DEFAULT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.progress ALTER COLUMN id SET DEFAULT nextval('public.progress_id_seq'::regclass);


--
-- Name: push_subscriptions id; Type: DEFAULT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.push_subscriptions ALTER COLUMN id SET DEFAULT nextval('public.push_subscriptions_id_seq'::regclass);


--
-- Name: reports id; Type: DEFAULT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.reports ALTER COLUMN id SET DEFAULT nextval('public.reports_id_seq'::regclass);


--
-- Name: scheduler_allocations id; Type: DEFAULT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.scheduler_allocations ALTER COLUMN id SET DEFAULT nextval('public.scheduler_allocations_id_seq'::regclass);


--
-- Name: scheduler_job_day_notes id; Type: DEFAULT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.scheduler_job_day_notes ALTER COLUMN id SET DEFAULT nextval('public.scheduler_job_day_notes_id_seq'::regclass);


--
-- Name: scheduler_maintenance_jobs id; Type: DEFAULT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.scheduler_maintenance_jobs ALTER COLUMN id SET DEFAULT nextval('public.scheduler_maintenance_jobs_id_seq'::regclass);


--
-- Name: scheduler_worker_absences id; Type: DEFAULT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.scheduler_worker_absences ALTER COLUMN id SET DEFAULT nextval('public.scheduler_worker_absences_id_seq'::regclass);


--
-- Name: scheduler_workers id; Type: DEFAULT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.scheduler_workers ALTER COLUMN id SET DEFAULT nextval('public.scheduler_workers_id_seq'::regclass);


--
-- Name: site_diary id; Type: DEFAULT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.site_diary ALTER COLUMN id SET DEFAULT nextval('public.site_diary_id_seq'::regclass);


--
-- Name: user_notification_prefs id; Type: DEFAULT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.user_notification_prefs ALTER COLUMN id SET DEFAULT nextval('public.user_notification_prefs_id_seq'::regclass);


--
-- Name: user_permissions id; Type: DEFAULT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.user_permissions ALTER COLUMN id SET DEFAULT nextval('public.user_permissions_id_seq'::regclass);


--
-- Name: users id; Type: DEFAULT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.users ALTER COLUMN id SET DEFAULT nextval('public.users_id_seq'::regclass);


--
-- Name: weather_snapshots id; Type: DEFAULT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.weather_snapshots ALTER COLUMN id SET DEFAULT nextval('public.weather_snapshots_id_seq'::regclass);


--
-- Name: _schema_version _schema_version_pkey; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public._schema_version
    ADD CONSTRAINT _schema_version_pkey PRIMARY KEY (id);


--
-- Name: ai_usage_daily ai_usage_daily_pkey; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.ai_usage_daily
    ADD CONSTRAINT ai_usage_daily_pkey PRIMARY KEY (id);


--
-- Name: ai_usage_daily ai_usage_daily_user_id_date_unique; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.ai_usage_daily
    ADD CONSTRAINT ai_usage_daily_user_id_date_unique UNIQUE (user_id, date);


--
-- Name: billing_webhook_notification_recipients billing_webhook_notification__stripe_event_id_recipient_ema_key; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.billing_webhook_notification_recipients
    ADD CONSTRAINT billing_webhook_notification__stripe_event_id_recipient_ema_key UNIQUE (stripe_event_id, recipient_email);


--
-- Name: billing_webhook_notification_recipients billing_webhook_notification_recipients_pkey; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.billing_webhook_notification_recipients
    ADD CONSTRAINT billing_webhook_notification_recipients_pkey PRIMARY KEY (id);


--
-- Name: billing_webhook_notifications billing_webhook_notifications_pkey; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.billing_webhook_notifications
    ADD CONSTRAINT billing_webhook_notifications_pkey PRIMARY KEY (stripe_event_id);


--
-- Name: call_forward call_forward_pkey; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.call_forward
    ADD CONSTRAINT call_forward_pkey PRIMARY KEY (id);


--
-- Name: call_forward_templates call_forward_templates_pkey; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.call_forward_templates
    ADD CONSTRAINT call_forward_templates_pkey PRIMARY KEY (id);


--
-- Name: companies companies_pkey; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.companies
    ADD CONSTRAINT companies_pkey PRIMARY KEY (id);


--
-- Name: company_branding company_branding_company_id_unique; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.company_branding
    ADD CONSTRAINT company_branding_company_id_unique UNIQUE (company_id);


--
-- Name: company_branding company_branding_pkey; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.company_branding
    ADD CONSTRAINT company_branding_pkey PRIMARY KEY (id);


--
-- Name: diary_media diary_media_pkey; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.diary_media
    ADD CONSTRAINT diary_media_pkey PRIMARY KEY (id);


--
-- Name: diary_note_comments diary_note_comments_pkey; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.diary_note_comments
    ADD CONSTRAINT diary_note_comments_pkey PRIMARY KEY (id);


--
-- Name: diary_notes diary_notes_pkey; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.diary_notes
    ADD CONSTRAINT diary_notes_pkey PRIMARY KEY (id);


--
-- Name: email_settings email_settings_company_id_unique; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.email_settings
    ADD CONSTRAINT email_settings_company_id_unique UNIQUE (company_id);


--
-- Name: email_settings email_settings_pkey; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.email_settings
    ADD CONSTRAINT email_settings_pkey PRIMARY KEY (id);


--
-- Name: eto_job_sequences eto_job_sequences_pkey; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.eto_job_sequences
    ADD CONSTRAINT eto_job_sequences_pkey PRIMARY KEY (job_id);


--
-- Name: inspection_form_items inspection_form_items_client_key_unique; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.inspection_form_items
    ADD CONSTRAINT inspection_form_items_client_key_unique UNIQUE (form_id, client_key);


--
-- Name: inspection_form_items inspection_form_items_pkey; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.inspection_form_items
    ADD CONSTRAINT inspection_form_items_pkey PRIMARY KEY (id);


--
-- Name: inspection_form_media inspection_form_media_pkey; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.inspection_form_media
    ADD CONSTRAINT inspection_form_media_pkey PRIMARY KEY (id);


--
-- Name: inspection_forms inspection_forms_pkey; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.inspection_forms
    ADD CONSTRAINT inspection_forms_pkey PRIMARY KEY (id);


--
-- Name: inspection_forms inspection_forms_user_job_unique; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.inspection_forms
    ADD CONSTRAINT inspection_forms_user_job_unique UNIQUE (company_id, job_id, created_by);


--
-- Name: job_assignments job_assignments_pkey; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.job_assignments
    ADD CONSTRAINT job_assignments_pkey PRIMARY KEY (id);


--
-- Name: job_dropbox_auto_uploads job_dropbox_auto_uploads_job_id_key; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.job_dropbox_auto_uploads
    ADD CONSTRAINT job_dropbox_auto_uploads_job_id_key UNIQUE (job_id);


--
-- Name: job_dropbox_auto_uploads job_dropbox_auto_uploads_pkey; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.job_dropbox_auto_uploads
    ADD CONSTRAINT job_dropbox_auto_uploads_pkey PRIMARY KEY (id);


--
-- Name: job_dropbox_folders job_dropbox_folders_pkey; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.job_dropbox_folders
    ADD CONSTRAINT job_dropbox_folders_pkey PRIMARY KEY (id);


--
-- Name: job_dropbox_uploads job_dropbox_uploads_pkey; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.job_dropbox_uploads
    ADD CONSTRAINT job_dropbox_uploads_pkey PRIMARY KEY (id);


--
-- Name: job_media job_media_pkey; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.job_media
    ADD CONSTRAINT job_media_pkey PRIMARY KEY (id);


--
-- Name: job_tasks job_tasks_pkey; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.job_tasks
    ADD CONSTRAINT job_tasks_pkey PRIMARY KEY (id);


--
-- Name: jobs jobs_company_job_number_unique; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.jobs
    ADD CONSTRAINT jobs_company_job_number_unique UNIQUE (company_id, job_number);


--
-- Name: jobs jobs_pkey; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.jobs
    ADD CONSTRAINT jobs_pkey PRIMARY KEY (id);


--
-- Name: media_deletion_queue media_deletion_queue_pkey; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.media_deletion_queue
    ADD CONSTRAINT media_deletion_queue_pkey PRIMARY KEY (id);


--
-- Name: notifications notifications_pkey; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.notifications
    ADD CONSTRAINT notifications_pkey PRIMARY KEY (id);


--
-- Name: password_reset_tokens password_reset_tokens_pkey; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.password_reset_tokens
    ADD CONSTRAINT password_reset_tokens_pkey PRIMARY KEY (id);


--
-- Name: password_reset_tokens password_reset_tokens_token_unique; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.password_reset_tokens
    ADD CONSTRAINT password_reset_tokens_token_unique UNIQUE (token);


--
-- Name: progress progress_pkey; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.progress
    ADD CONSTRAINT progress_pkey PRIMARY KEY (id);


--
-- Name: push_subscriptions push_subscriptions_endpoint_key; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.push_subscriptions
    ADD CONSTRAINT push_subscriptions_endpoint_key UNIQUE (endpoint);


--
-- Name: push_subscriptions push_subscriptions_pkey; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.push_subscriptions
    ADD CONSTRAINT push_subscriptions_pkey PRIMARY KEY (id);


--
-- Name: reports reports_pkey; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.reports
    ADD CONSTRAINT reports_pkey PRIMARY KEY (id);


--
-- Name: scheduler_allocations scheduler_allocations_pkey; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.scheduler_allocations
    ADD CONSTRAINT scheduler_allocations_pkey PRIMARY KEY (id);


--
-- Name: scheduler_job_day_notes scheduler_job_day_notes_pkey; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.scheduler_job_day_notes
    ADD CONSTRAINT scheduler_job_day_notes_pkey PRIMARY KEY (id);


--
-- Name: scheduler_maintenance_jobs scheduler_maintenance_jobs_pkey; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.scheduler_maintenance_jobs
    ADD CONSTRAINT scheduler_maintenance_jobs_pkey PRIMARY KEY (id);


--
-- Name: scheduler_worker_absences scheduler_worker_absences_pkey; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.scheduler_worker_absences
    ADD CONSTRAINT scheduler_worker_absences_pkey PRIMARY KEY (id);


--
-- Name: scheduler_workers scheduler_workers_pkey; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.scheduler_workers
    ADD CONSTRAINT scheduler_workers_pkey PRIMARY KEY (id);


--
-- Name: site_diary site_diary_pkey; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.site_diary
    ADD CONSTRAINT site_diary_pkey PRIMARY KEY (id);


--
-- Name: supabase_import_mappings supabase_import_mappings_pkey; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.supabase_import_mappings
    ADD CONSTRAINT supabase_import_mappings_pkey PRIMARY KEY (source_origin, entity, source_id);


--
-- Name: supabase_import_mappings supabase_import_mappings_source_origin_entity_target_id_key; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.supabase_import_mappings
    ADD CONSTRAINT supabase_import_mappings_source_origin_entity_target_id_key UNIQUE (source_origin, entity, target_id);


--
-- Name: job_dropbox_uploads uq_job_dropbox_upload_media; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.job_dropbox_uploads
    ADD CONSTRAINT uq_job_dropbox_upload_media UNIQUE (job_id, folder_id, media_source, media_id);


--
-- Name: scheduler_allocations uq_scheduler_company_worker_date; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.scheduler_allocations
    ADD CONSTRAINT uq_scheduler_company_worker_date UNIQUE (company_id, worker_id, assigned_date);


--
-- Name: scheduler_job_day_notes uq_scheduler_job_day_note; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.scheduler_job_day_notes
    ADD CONSTRAINT uq_scheduler_job_day_note UNIQUE (job_id, note_date);


--
-- Name: scheduler_worker_absences uq_scheduler_worker_absence_period; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.scheduler_worker_absences
    ADD CONSTRAINT uq_scheduler_worker_absence_period UNIQUE (worker_id, absence_type, start_date, end_date);


--
-- Name: scheduler_allocations uq_scheduler_worker_job_date; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.scheduler_allocations
    ADD CONSTRAINT uq_scheduler_worker_job_date UNIQUE (worker_id, job_id, assigned_date);


--
-- Name: user_notification_prefs user_notification_prefs_pkey; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.user_notification_prefs
    ADD CONSTRAINT user_notification_prefs_pkey PRIMARY KEY (id);


--
-- Name: user_notification_prefs user_notification_prefs_user_id_key; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.user_notification_prefs
    ADD CONSTRAINT user_notification_prefs_user_id_key UNIQUE (user_id);


--
-- Name: user_permissions user_permissions_pkey; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.user_permissions
    ADD CONSTRAINT user_permissions_pkey PRIMARY KEY (id);


--
-- Name: user_permissions user_permissions_user_id_resource_action_unique; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.user_permissions
    ADD CONSTRAINT user_permissions_user_id_resource_action_unique UNIQUE (user_id, resource, action);


--
-- Name: users users_email_unique; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.users
    ADD CONSTRAINT users_email_unique UNIQUE (email);


--
-- Name: users users_pkey; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.users
    ADD CONSTRAINT users_pkey PRIMARY KEY (id);


--
-- Name: weather_snapshots weather_snapshots_pkey; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.weather_snapshots
    ADD CONSTRAINT weather_snapshots_pkey PRIMARY KEY (id);


--
-- Name: billing_webhook_recipient_retry_due_idx; Type: INDEX; Schema: public; Owner: -
--

CREATE INDEX billing_webhook_recipient_retry_due_idx ON public.billing_webhook_notification_recipients USING btree (next_attempt_at, id) WHERE (status = ANY (ARRAY['pending'::text, 'failed'::text, 'processing'::text]));


--
-- Name: idx_inspection_form_items_form; Type: INDEX; Schema: public; Owner: -
--

CREATE INDEX idx_inspection_form_items_form ON public.inspection_form_items USING btree (form_id, sort_order);


--
-- Name: idx_inspection_form_media_form; Type: INDEX; Schema: public; Owner: -
--

CREATE INDEX idx_inspection_form_media_form ON public.inspection_form_media USING btree (form_id, item_id, sort_order);


--
-- Name: idx_job_media_job_created; Type: INDEX; Schema: public; Owner: -
--

CREATE INDEX idx_job_media_job_created ON public.job_media USING btree (job_id, created_at DESC);


--
-- Name: idx_notifications_backup_warning; Type: INDEX; Schema: public; Owner: -
--

CREATE INDEX idx_notifications_backup_warning ON public.notifications USING btree (user_id, type, link, created_at) WHERE (resolved_at IS NULL);


--
-- Name: idx_scheduler_allocations_company_date; Type: INDEX; Schema: public; Owner: -
--

CREATE INDEX idx_scheduler_allocations_company_date ON public.scheduler_allocations USING btree (company_id, assigned_date);


--
-- Name: idx_scheduler_worker_absences_company_dates; Type: INDEX; Schema: public; Owner: -
--

CREATE INDEX idx_scheduler_worker_absences_company_dates ON public.scheduler_worker_absences USING btree (company_id, start_date, end_date);


--
-- Name: idx_scheduler_worker_absences_worker_dates; Type: INDEX; Schema: public; Owner: -
--

CREATE INDEX idx_scheduler_worker_absences_worker_dates ON public.scheduler_worker_absences USING btree (worker_id, start_date, end_date);


--
-- Name: uq_media_deletion_queue_stored_name; Type: INDEX; Schema: public; Owner: -
--

CREATE UNIQUE INDEX uq_media_deletion_queue_stored_name ON public.media_deletion_queue USING btree (stored_name);


--
-- Name: uq_scheduler_worker_maintenance_job_date; Type: INDEX; Schema: public; Owner: -
--

CREATE UNIQUE INDEX uq_scheduler_worker_maintenance_job_date ON public.scheduler_allocations USING btree (worker_id, maintenance_job_id, assigned_date) WHERE (maintenance_job_id IS NOT NULL);


--
-- Name: scheduler_worker_absences scheduler_worker_absences_no_overlap_trigger; Type: TRIGGER; Schema: public; Owner: -
--

CREATE TRIGGER scheduler_worker_absences_no_overlap_trigger BEFORE INSERT OR UPDATE OF company_id, worker_id, start_date, end_date ON public.scheduler_worker_absences FOR EACH ROW EXECUTE FUNCTION public.enforce_scheduler_worker_absence_no_overlap();


--
-- Name: billing_webhook_notification_recipients billing_webhook_notification_recipients_stripe_event_id_fkey; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.billing_webhook_notification_recipients
    ADD CONSTRAINT billing_webhook_notification_recipients_stripe_event_id_fkey FOREIGN KEY (stripe_event_id) REFERENCES public.billing_webhook_notifications(stripe_event_id) ON DELETE CASCADE;


--
-- Name: call_forward call_forward_job_id_jobs_id_fk; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.call_forward
    ADD CONSTRAINT call_forward_job_id_jobs_id_fk FOREIGN KEY (job_id) REFERENCES public.jobs(id) ON DELETE CASCADE;


--
-- Name: call_forward_templates call_forward_templates_company_id_companies_id_fk; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.call_forward_templates
    ADD CONSTRAINT call_forward_templates_company_id_companies_id_fk FOREIGN KEY (company_id) REFERENCES public.companies(id) ON DELETE CASCADE;


--
-- Name: company_branding company_branding_company_id_companies_id_fk; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.company_branding
    ADD CONSTRAINT company_branding_company_id_companies_id_fk FOREIGN KEY (company_id) REFERENCES public.companies(id) ON DELETE CASCADE;


--
-- Name: diary_media diary_media_diary_entry_id_site_diary_id_fk; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.diary_media
    ADD CONSTRAINT diary_media_diary_entry_id_site_diary_id_fk FOREIGN KEY (diary_entry_id) REFERENCES public.site_diary(id) ON DELETE CASCADE;


--
-- Name: diary_media diary_media_note_id_diary_notes_id_fk; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.diary_media
    ADD CONSTRAINT diary_media_note_id_diary_notes_id_fk FOREIGN KEY (note_id) REFERENCES public.diary_notes(id) ON DELETE CASCADE;


--
-- Name: diary_media diary_media_uploaded_by_users_id_fk; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.diary_media
    ADD CONSTRAINT diary_media_uploaded_by_users_id_fk FOREIGN KEY (uploaded_by) REFERENCES public.users(id) ON DELETE SET NULL;


--
-- Name: diary_note_comments diary_note_comments_author_id_users_id_fk; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.diary_note_comments
    ADD CONSTRAINT diary_note_comments_author_id_users_id_fk FOREIGN KEY (author_id) REFERENCES public.users(id) ON DELETE SET NULL;


--
-- Name: diary_note_comments diary_note_comments_note_id_diary_notes_id_fk; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.diary_note_comments
    ADD CONSTRAINT diary_note_comments_note_id_diary_notes_id_fk FOREIGN KEY (note_id) REFERENCES public.diary_notes(id) ON DELETE CASCADE;


--
-- Name: diary_notes diary_notes_action_raised_by_users_id_fk; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.diary_notes
    ADD CONSTRAINT diary_notes_action_raised_by_users_id_fk FOREIGN KEY (action_raised_by) REFERENCES public.users(id) ON DELETE SET NULL;


--
-- Name: diary_notes diary_notes_diary_entry_id_site_diary_id_fk; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.diary_notes
    ADD CONSTRAINT diary_notes_diary_entry_id_site_diary_id_fk FOREIGN KEY (diary_entry_id) REFERENCES public.site_diary(id) ON DELETE CASCADE;


--
-- Name: email_settings email_settings_company_id_companies_id_fk; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.email_settings
    ADD CONSTRAINT email_settings_company_id_companies_id_fk FOREIGN KEY (company_id) REFERENCES public.companies(id) ON DELETE CASCADE;


--
-- Name: eto_job_sequences eto_job_sequences_job_id_fkey; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.eto_job_sequences
    ADD CONSTRAINT eto_job_sequences_job_id_fkey FOREIGN KEY (job_id) REFERENCES public.jobs(id) ON DELETE CASCADE;


--
-- Name: inspection_form_items inspection_form_items_form_id_fkey; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.inspection_form_items
    ADD CONSTRAINT inspection_form_items_form_id_fkey FOREIGN KEY (form_id) REFERENCES public.inspection_forms(id) ON DELETE CASCADE;


--
-- Name: inspection_form_media inspection_form_media_diary_media_id_fkey; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.inspection_form_media
    ADD CONSTRAINT inspection_form_media_diary_media_id_fkey FOREIGN KEY (diary_media_id) REFERENCES public.diary_media(id) ON DELETE CASCADE;


--
-- Name: inspection_form_media inspection_form_media_form_id_fkey; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.inspection_form_media
    ADD CONSTRAINT inspection_form_media_form_id_fkey FOREIGN KEY (form_id) REFERENCES public.inspection_forms(id) ON DELETE CASCADE;


--
-- Name: inspection_form_media inspection_form_media_item_id_fkey; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.inspection_form_media
    ADD CONSTRAINT inspection_form_media_item_id_fkey FOREIGN KEY (item_id) REFERENCES public.inspection_form_items(id) ON DELETE CASCADE;


--
-- Name: inspection_forms inspection_forms_company_id_fkey; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.inspection_forms
    ADD CONSTRAINT inspection_forms_company_id_fkey FOREIGN KEY (company_id) REFERENCES public.companies(id) ON DELETE CASCADE;


--
-- Name: inspection_forms inspection_forms_created_by_fkey; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.inspection_forms
    ADD CONSTRAINT inspection_forms_created_by_fkey FOREIGN KEY (created_by) REFERENCES public.users(id) ON DELETE CASCADE;


--
-- Name: inspection_forms inspection_forms_diary_entry_id_fkey; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.inspection_forms
    ADD CONSTRAINT inspection_forms_diary_entry_id_fkey FOREIGN KEY (diary_entry_id) REFERENCES public.site_diary(id) ON DELETE CASCADE;


--
-- Name: inspection_forms inspection_forms_diary_note_id_fkey; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.inspection_forms
    ADD CONSTRAINT inspection_forms_diary_note_id_fkey FOREIGN KEY (diary_note_id) REFERENCES public.diary_notes(id) ON DELETE CASCADE;


--
-- Name: inspection_forms inspection_forms_job_id_fkey; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.inspection_forms
    ADD CONSTRAINT inspection_forms_job_id_fkey FOREIGN KEY (job_id) REFERENCES public.jobs(id) ON DELETE CASCADE;


--
-- Name: job_assignments job_assignments_job_id_jobs_id_fk; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.job_assignments
    ADD CONSTRAINT job_assignments_job_id_jobs_id_fk FOREIGN KEY (job_id) REFERENCES public.jobs(id) ON DELETE CASCADE;


--
-- Name: job_assignments job_assignments_user_id_users_id_fk; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.job_assignments
    ADD CONSTRAINT job_assignments_user_id_users_id_fk FOREIGN KEY (user_id) REFERENCES public.users(id) ON DELETE CASCADE;


--
-- Name: job_dropbox_auto_uploads job_dropbox_auto_uploads_company_id_fkey; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.job_dropbox_auto_uploads
    ADD CONSTRAINT job_dropbox_auto_uploads_company_id_fkey FOREIGN KEY (company_id) REFERENCES public.companies(id) ON DELETE CASCADE;


--
-- Name: job_dropbox_auto_uploads job_dropbox_auto_uploads_folder_id_fkey; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.job_dropbox_auto_uploads
    ADD CONSTRAINT job_dropbox_auto_uploads_folder_id_fkey FOREIGN KEY (folder_id) REFERENCES public.job_dropbox_folders(id) ON DELETE CASCADE;


--
-- Name: job_dropbox_auto_uploads job_dropbox_auto_uploads_job_id_fkey; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.job_dropbox_auto_uploads
    ADD CONSTRAINT job_dropbox_auto_uploads_job_id_fkey FOREIGN KEY (job_id) REFERENCES public.jobs(id) ON DELETE CASCADE;


--
-- Name: job_dropbox_folders job_dropbox_folders_company_id_companies_id_fk; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.job_dropbox_folders
    ADD CONSTRAINT job_dropbox_folders_company_id_companies_id_fk FOREIGN KEY (company_id) REFERENCES public.companies(id) ON DELETE CASCADE;


--
-- Name: job_dropbox_folders job_dropbox_folders_job_id_jobs_id_fk; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.job_dropbox_folders
    ADD CONSTRAINT job_dropbox_folders_job_id_jobs_id_fk FOREIGN KEY (job_id) REFERENCES public.jobs(id) ON DELETE CASCADE;


--
-- Name: job_dropbox_uploads job_dropbox_uploads_company_id_fkey; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.job_dropbox_uploads
    ADD CONSTRAINT job_dropbox_uploads_company_id_fkey FOREIGN KEY (company_id) REFERENCES public.companies(id) ON DELETE CASCADE;


--
-- Name: job_dropbox_uploads job_dropbox_uploads_folder_id_fkey; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.job_dropbox_uploads
    ADD CONSTRAINT job_dropbox_uploads_folder_id_fkey FOREIGN KEY (folder_id) REFERENCES public.job_dropbox_folders(id) ON DELETE CASCADE;


--
-- Name: job_dropbox_uploads job_dropbox_uploads_job_id_fkey; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.job_dropbox_uploads
    ADD CONSTRAINT job_dropbox_uploads_job_id_fkey FOREIGN KEY (job_id) REFERENCES public.jobs(id) ON DELETE CASCADE;


--
-- Name: job_media job_media_job_id_fkey; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.job_media
    ADD CONSTRAINT job_media_job_id_fkey FOREIGN KEY (job_id) REFERENCES public.jobs(id) ON DELETE CASCADE;


--
-- Name: job_media job_media_uploaded_by_fkey; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.job_media
    ADD CONSTRAINT job_media_uploaded_by_fkey FOREIGN KEY (uploaded_by) REFERENCES public.users(id) ON DELETE SET NULL;


--
-- Name: job_tasks job_tasks_job_id_jobs_id_fk; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.job_tasks
    ADD CONSTRAINT job_tasks_job_id_jobs_id_fk FOREIGN KEY (job_id) REFERENCES public.jobs(id) ON DELETE CASCADE;


--
-- Name: jobs jobs_company_id_companies_id_fk; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.jobs
    ADD CONSTRAINT jobs_company_id_companies_id_fk FOREIGN KEY (company_id) REFERENCES public.companies(id) ON DELETE CASCADE;


--
-- Name: jobs jobs_manager_id_users_id_fk; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.jobs
    ADD CONSTRAINT jobs_manager_id_users_id_fk FOREIGN KEY (manager_id) REFERENCES public.users(id) ON DELETE SET NULL;


--
-- Name: jobs jobs_supervisor_id_users_id_fk; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.jobs
    ADD CONSTRAINT jobs_supervisor_id_users_id_fk FOREIGN KEY (supervisor_id) REFERENCES public.users(id) ON DELETE SET NULL;


--
-- Name: notifications notifications_user_id_users_id_fk; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.notifications
    ADD CONSTRAINT notifications_user_id_users_id_fk FOREIGN KEY (user_id) REFERENCES public.users(id) ON DELETE CASCADE;


--
-- Name: password_reset_tokens password_reset_tokens_user_id_users_id_fk; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.password_reset_tokens
    ADD CONSTRAINT password_reset_tokens_user_id_users_id_fk FOREIGN KEY (user_id) REFERENCES public.users(id) ON DELETE CASCADE;


--
-- Name: progress progress_job_id_jobs_id_fk; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.progress
    ADD CONSTRAINT progress_job_id_jobs_id_fk FOREIGN KEY (job_id) REFERENCES public.jobs(id) ON DELETE CASCADE;


--
-- Name: push_subscriptions push_subscriptions_user_id_fkey; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.push_subscriptions
    ADD CONSTRAINT push_subscriptions_user_id_fkey FOREIGN KEY (user_id) REFERENCES public.users(id) ON DELETE CASCADE;


--
-- Name: reports reports_job_id_jobs_id_fk; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.reports
    ADD CONSTRAINT reports_job_id_jobs_id_fk FOREIGN KEY (job_id) REFERENCES public.jobs(id) ON DELETE CASCADE;


--
-- Name: scheduler_allocations scheduler_allocations_company_id_companies_id_fk; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.scheduler_allocations
    ADD CONSTRAINT scheduler_allocations_company_id_companies_id_fk FOREIGN KEY (company_id) REFERENCES public.companies(id) ON DELETE CASCADE;


--
-- Name: scheduler_allocations scheduler_allocations_job_id_jobs_id_fk; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.scheduler_allocations
    ADD CONSTRAINT scheduler_allocations_job_id_jobs_id_fk FOREIGN KEY (job_id) REFERENCES public.jobs(id) ON DELETE CASCADE;


--
-- Name: scheduler_allocations scheduler_allocations_maintenance_job_id_fkey; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.scheduler_allocations
    ADD CONSTRAINT scheduler_allocations_maintenance_job_id_fkey FOREIGN KEY (maintenance_job_id) REFERENCES public.scheduler_maintenance_jobs(id) ON DELETE CASCADE;


--
-- Name: scheduler_allocations scheduler_allocations_worker_id_scheduler_workers_id_fk; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.scheduler_allocations
    ADD CONSTRAINT scheduler_allocations_worker_id_scheduler_workers_id_fk FOREIGN KEY (worker_id) REFERENCES public.scheduler_workers(id) ON DELETE CASCADE;


--
-- Name: scheduler_job_day_notes scheduler_job_day_notes_company_id_companies_id_fk; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.scheduler_job_day_notes
    ADD CONSTRAINT scheduler_job_day_notes_company_id_companies_id_fk FOREIGN KEY (company_id) REFERENCES public.companies(id) ON DELETE CASCADE;


--
-- Name: scheduler_job_day_notes scheduler_job_day_notes_job_id_jobs_id_fk; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.scheduler_job_day_notes
    ADD CONSTRAINT scheduler_job_day_notes_job_id_jobs_id_fk FOREIGN KEY (job_id) REFERENCES public.jobs(id) ON DELETE CASCADE;


--
-- Name: scheduler_maintenance_jobs scheduler_maintenance_jobs_company_id_fkey; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.scheduler_maintenance_jobs
    ADD CONSTRAINT scheduler_maintenance_jobs_company_id_fkey FOREIGN KEY (company_id) REFERENCES public.companies(id) ON DELETE CASCADE;


--
-- Name: scheduler_worker_absences scheduler_worker_absences_company_id_fkey; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.scheduler_worker_absences
    ADD CONSTRAINT scheduler_worker_absences_company_id_fkey FOREIGN KEY (company_id) REFERENCES public.companies(id) ON DELETE CASCADE;


--
-- Name: scheduler_worker_absences scheduler_worker_absences_worker_id_fkey; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.scheduler_worker_absences
    ADD CONSTRAINT scheduler_worker_absences_worker_id_fkey FOREIGN KEY (worker_id) REFERENCES public.scheduler_workers(id) ON DELETE CASCADE;


--
-- Name: scheduler_workers scheduler_workers_company_id_companies_id_fk; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.scheduler_workers
    ADD CONSTRAINT scheduler_workers_company_id_companies_id_fk FOREIGN KEY (company_id) REFERENCES public.companies(id) ON DELETE CASCADE;


--
-- Name: site_diary site_diary_action_raised_by_users_id_fk; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.site_diary
    ADD CONSTRAINT site_diary_action_raised_by_users_id_fk FOREIGN KEY (action_raised_by) REFERENCES public.users(id) ON DELETE SET NULL;


--
-- Name: site_diary site_diary_author_id_users_id_fk; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.site_diary
    ADD CONSTRAINT site_diary_author_id_users_id_fk FOREIGN KEY (author_id) REFERENCES public.users(id) ON DELETE SET NULL;


--
-- Name: site_diary site_diary_job_id_jobs_id_fk; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.site_diary
    ADD CONSTRAINT site_diary_job_id_jobs_id_fk FOREIGN KEY (job_id) REFERENCES public.jobs(id) ON DELETE CASCADE;


--
-- Name: user_notification_prefs user_notification_prefs_user_id_fkey; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.user_notification_prefs
    ADD CONSTRAINT user_notification_prefs_user_id_fkey FOREIGN KEY (user_id) REFERENCES public.users(id) ON DELETE CASCADE;


--
-- Name: user_permissions user_permissions_user_id_users_id_fk; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.user_permissions
    ADD CONSTRAINT user_permissions_user_id_users_id_fk FOREIGN KEY (user_id) REFERENCES public.users(id) ON DELETE CASCADE;


--
-- Name: users users_company_id_companies_id_fk; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.users
    ADD CONSTRAINT users_company_id_companies_id_fk FOREIGN KEY (company_id) REFERENCES public.companies(id) ON DELETE CASCADE;


--
-- Name: weather_snapshots weather_snapshots_diary_entry_id_site_diary_id_fk; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.weather_snapshots
    ADD CONSTRAINT weather_snapshots_diary_entry_id_site_diary_id_fk FOREIGN KEY (diary_entry_id) REFERENCES public.site_diary(id) ON DELETE CASCADE;


--
-- PostgreSQL database dump complete
--

\unrestrict S5SXzHd7jIfsxYrSAAlaiHDYyvBFj4KkmSRBZzGWIlTfeFVa3R7kMKkzhHIyHZ7

